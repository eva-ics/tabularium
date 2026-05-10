//! Database facade: `Storage` + Tantivy + read-through cache (`moka`).

mod entry_kind;
mod meta;
mod ops;
mod search_index;
mod sqlite;
mod storage;
mod time_parse;

pub use entry_kind::EntryKind;
pub use meta::{DocumentMeta, GrepLine, ListedEntry, SearchHit, WcStats};
use search_index::SearchIndex;
pub use sqlite::SqliteStorage;
pub use storage::Storage;
pub use time_parse::parse_user_timestamp;

use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use moka::future::Cache;
use tokio::sync::{Mutex, watch as wait_cell};
use tracing::instrument;

use crate::acl::AuthContext;
use crate::resource_path::{canonical_path_segments, parent_and_final_name};
use crate::validation::validate_entity_name;
use crate::{Error, Result, Timestamp};

/// Opaque entry primary key (directory or file row in `entries`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct EntryId(i64);

impl EntryId {
    /// Raw sqlite row id.
    pub fn raw(self) -> i64 {
        self.0
    }

    pub(crate) const fn from_raw(id: i64) -> Self {
        Self(id)
    }
}

impl fmt::Display for EntryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<i64> for EntryId {
    fn from(id: i64) -> Self {
        Self::from_raw(id)
    }
}

impl From<EntryId> for i64 {
    fn from(id: EntryId) -> Self {
        id.raw()
    }
}

/// Result of waiting on a document until content changes (long poll).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentWaitStatus {
    /// A content-changing write landed after the wait began.
    Changed,
    /// No such write before the configured timeout.
    TimedOut,
}

/// Async façade over storage, search index, and document body cache.
pub struct Database<S: Storage> {
    storage: S,
    search: SearchIndex,
    cache: Cache<EntryId, String>,
    cache_active: bool,
    doc_wait: Arc<DashMap<EntryId, wait_cell::Sender<u64>>>,
    /// Per-file mutex for atomic read-check-append (`append_if_not_contains_by_path`).
    doc_append_locks: Arc<DashMap<EntryId, Arc<Mutex<()>>>>,
}

/// Type alias for the stage-1 sqlite stack.
pub type SqliteDatabase = Database<SqliteStorage>;

impl SqliteDatabase {
    /// Opens SQLite at `db_uri`, Tantivy at `index_path`, and configures the body cache.
    ///
    /// `cache_size` is the max number of cached document bodies; `0` keeps a cache handle but
    /// disables population (always loads from storage).
    #[instrument(
        skip(db_uri, index_path),
        fields(
            db_uri = db_uri.as_ref(),
            index_path = %index_path.as_ref().display(),
            cache_size,
        ),
        err(Debug)
    )]
    pub async fn init(
        db_uri: impl AsRef<str>,
        index_path: impl AsRef<Path>,
        cache_size: u64,
    ) -> Result<Self> {
        let storage = SqliteStorage::connect(db_uri.as_ref()).await?;
        let search = SearchIndex::open(index_path.as_ref())?;
        let cache = Cache::builder().max_capacity(cache_size.max(1)).build();
        Ok(Database {
            storage,
            search,
            cache,
            cache_active: cache_size > 0,
            doc_wait: Arc::new(DashMap::new()),
            doc_append_locks: Arc::new(DashMap::new()),
        })
    }

    pub async fn acl_list_rows(&self) -> Result<Vec<(String, String)>> {
        self.storage.acl_list_rows().await
    }

    pub async fn acl_get_json(&self, name: &str) -> Result<String> {
        self.storage.acl_get_json(name).await
    }

    pub async fn acl_upsert_validated(&self, name: &str, body_json: &str) -> Result<()> {
        self.storage.acl_upsert_validated(name, body_json).await
    }

    pub async fn acl_delete_named(&self, name: &str) -> Result<()> {
        self.storage.acl_delete_named(name).await
    }

    pub async fn resolve_auth_key(&self, key: &str) -> Result<Option<AuthContext>> {
        let Some((name, json)) = self.storage.auth_lookup_key(key).await? else {
            return Ok(None);
        };
        let body = crate::parse_acl_json(&json)?;
        Ok(Some(AuthContext::new(name, body)))
    }

    pub async fn psk_list_rows(&self) -> Result<Vec<(String, String, String)>> {
        self.storage.psk_list_rows().await
    }

    pub async fn psk_insert(&self, psk_name: &str, acl_name: &str, key: &str) -> Result<()> {
        self.storage.psk_insert(psk_name, acl_name, key).await
    }

    pub async fn psk_delete_named(&self, psk_name: &str) -> Result<()> {
        self.storage.psk_delete_named(psk_name).await
    }
}

impl<S: Storage> Database<S> {
    pub(crate) fn doc_append_mutex(&self, file_id: EntryId) -> Arc<Mutex<()>> {
        self.doc_append_locks
            .entry(file_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    async fn refresh_search_subtree(&self, directory_path: &str) -> Result<()> {
        let rows = self
            .storage
            .files_for_search_reindex(Some(directory_path))
            .await?;
        self.search.upsert_batch(&rows).await
    }

    async fn reindex_file_in_search(&self, file_id: EntryId) -> Result<()> {
        let (dp, name, desc, content) = self.storage.file_search_index_fields(file_id).await?;
        self.search
            .upsert_file(file_id, &dp, &name, &desc, &content)
            .await
    }

    fn bump_doc_wait(&self, file_id: EntryId) {
        if let Some(entry) = self.doc_wait.get(&file_id) {
            let tx = entry.value();
            let next = *tx.subscribe().borrow() + 1;
            let _ = tx.send(next);
        }
    }

    /// Long-poll until a content-changing write touches `file_id`, or `timeout` elapses.
    #[instrument(skip(self), fields(file_id = file_id.raw()), err(Debug))]
    pub async fn wait_until_document_changed(
        &self,
        file_id: EntryId,
        timeout: Duration,
    ) -> Result<DocumentWaitStatus> {
        self.storage.get_file_meta(file_id).await?;
        let tx = self
            .doc_wait
            .entry(file_id)
            .or_insert_with(|| {
                let (tx, _) = wait_cell::channel(0_u64);
                tx
            })
            .clone();
        let mut rx = tx.subscribe();
        tokio::select! {
            () = tokio::time::sleep(timeout) => Ok(DocumentWaitStatus::TimedOut),
            r = rx.changed() => {
                r.map_err(|_| crate::Error::InvalidInput("document wait closed".into()))?;
                Ok(DocumentWaitStatus::Changed)
            }
        }
    }

    /// Subscribe to content-change notifications for `file_id`.
    #[instrument(skip(self), fields(file_id = file_id.raw()), err(Debug))]
    pub async fn subscribe_document_wait(
        &self,
        file_id: EntryId,
    ) -> Result<wait_cell::Receiver<u64>> {
        self.storage.get_file_meta(file_id).await?;
        let tx = self
            .doc_wait
            .entry(file_id)
            .or_insert_with(|| {
                let (tx, _) = wait_cell::channel(0_u64);
                tx
            })
            .clone();
        Ok(tx.subscribe())
    }

    /// Expose `touch` for callers that already hold content elsewhere.
    #[instrument(skip(self), fields(file_id = file_id.raw()), err(Debug))]
    pub async fn touch(&self, file_id: EntryId) -> Result<()> {
        self.storage.touch(file_id).await
    }

    #[instrument(
        skip(self, description),
        fields(path = %path.as_ref(), parents = parents),
        err(Debug)
    )]
    pub async fn create_directory(
        &self,
        path: impl AsRef<str> + Send,
        description: Option<&str>,
        parents: bool,
    ) -> Result<EntryId> {
        let path = path.as_ref();
        canonical_path_segments(path)?;
        self.storage
            .create_directory(path, description, parents)
            .await
    }

    #[instrument(skip(self), fields(path = %path.as_ref()), err(Debug))]
    pub async fn entry_description(&self, path: impl AsRef<str> + Send) -> Result<Option<String>> {
        let path = path.as_ref();
        canonical_path_segments(path)?;
        self.storage.entry_description(path).await
    }

    #[instrument(skip(self, description), fields(path = %path.as_ref()), err(Debug))]
    pub async fn set_entry_description(
        &self,
        path: impl AsRef<str> + Send,
        description: Option<&str>,
    ) -> Result<()> {
        let path = path.as_ref();
        canonical_path_segments(path)?;
        self.storage
            .set_entry_description(path, description)
            .await?;
        if let Ok(fid) = self.storage.resolve_path(path, Some(EntryKind::File)).await {
            self.reindex_file_in_search(fid).await?;
        }
        Ok(())
    }

    #[instrument(skip(self), fields(path = %path.as_ref()), err(Debug))]
    pub async fn delete_directory(&self, path: impl AsRef<str> + Send) -> Result<()> {
        let path = path.as_ref();
        self.storage.delete_directory(path).await
    }

    #[instrument(skip(self), fields(path = %path.as_ref()), err(Debug))]
    pub async fn delete_directory_recursive(&self, path: impl AsRef<str> + Send) -> Result<()> {
        let path = path.as_ref();
        let rows = self.storage.files_for_search_reindex(Some(path)).await?;
        self.storage.delete_directory_recursive(path).await?;
        for (id, _, _, _, _) in rows {
            self.search.delete_file(id).await?;
            self.cache.invalidate(&id).await;
            self.doc_wait.remove(&id);
        }
        Ok(())
    }

    #[instrument(
        skip(self),
        fields(old_path = %old_path.as_ref(), new_path = %new_path.as_ref()),
        err(Debug)
    )]
    pub async fn rename_directory(
        &self,
        old_path: impl AsRef<str> + Send,
        new_path: impl AsRef<str> + Send,
    ) -> Result<()> {
        let old_path = old_path.as_ref();
        let new_path = new_path.as_ref();
        let (_, new_name) = parent_and_final_name(new_path)?;
        validate_entity_name(&new_name)?;
        self.storage.rename_directory(old_path, new_path).await?;
        self.refresh_search_subtree(new_path).await
    }

    #[instrument(skip(self, src_path, dst_parent, new_name), err(Debug))]
    pub async fn move_directory(
        &self,
        src_path: impl AsRef<str> + Send,
        dst_parent: impl AsRef<str> + Send,
        new_name: impl AsRef<str> + Send,
    ) -> Result<()> {
        let src_path = src_path.as_ref();
        let dst_parent = dst_parent.as_ref();
        let new_name = new_name.as_ref();
        validate_entity_name(new_name)?;
        let dir_id = self
            .storage
            .resolve_path(src_path, Some(EntryKind::Dir))
            .await?;
        self.storage
            .move_directory(dir_id, dst_parent, new_name)
            .await?;
        let new_full = if dst_parent == "/" {
            format!("/{new_name}")
        } else {
            format!("{dst_parent}/{new_name}")
        };
        self.refresh_search_subtree(&new_full).await
    }

    #[instrument(skip(self, directory_path, name, content), err(Debug))]
    pub async fn create_file_in_directory(
        &self,
        directory_path: impl AsRef<str> + Send,
        name: impl AsRef<str> + Send,
        content: impl AsRef<str> + Send,
    ) -> Result<EntryId> {
        validate_entity_name(name.as_ref())?;
        let dir = directory_path.as_ref().trim_end_matches('/');
        let full = if dir == "/" {
            format!("/{}", name.as_ref())
        } else {
            format!("{}/{}", dir, name.as_ref())
        };
        let id = self.storage.create_file(&full, content.as_ref()).await?;
        self.reindex_file_in_search(id).await?;
        Ok(id)
    }

    #[instrument(skip(self), fields(file_id = file_id.raw()), err(Debug))]
    pub async fn delete_document(&self, file_id: EntryId) -> Result<()> {
        self.storage.delete_file(file_id).await?;
        self.search.delete_file(file_id).await?;
        self.cache.invalidate(&file_id).await;
        self.doc_wait.remove(&file_id);
        self.doc_append_locks.remove(&file_id);
        Ok(())
    }

    #[instrument(
        skip(self, new_content),
        fields(file_id = file_id.raw(), new_len = new_content.as_ref().len()),
        err(Debug)
    )]
    pub async fn update_document(
        &self,
        file_id: EntryId,
        new_content: impl AsRef<str> + Send,
    ) -> Result<()> {
        self.storage
            .update_file(file_id, new_content.as_ref())
            .await?;
        self.reindex_file_in_search(file_id).await?;
        self.cache.invalidate(&file_id).await;
        self.bump_doc_wait(file_id);
        Ok(())
    }

    #[instrument(
        skip(self, new_content),
        fields(file_id = file_id.raw(), new_len = new_content.as_ref().len()),
        err(Debug)
    )]
    pub async fn update_document_if_revision(
        &self,
        file_id: EntryId,
        new_content: impl AsRef<str> + Send,
        expected_revision: &str,
    ) -> Result<()> {
        self.storage
            .update_file_if_revision_matches(file_id, new_content.as_ref(), expected_revision)
            .await?;
        self.reindex_file_in_search(file_id).await?;
        self.cache.invalidate(&file_id).await;
        self.bump_doc_wait(file_id);
        Ok(())
    }

    #[instrument(
        skip(self, to_append),
        fields(file_id = file_id.raw(), append_len = to_append.as_ref().len()),
        err(Debug)
    )]
    pub async fn append_document(
        &self,
        file_id: EntryId,
        to_append: impl AsRef<str> + Send,
    ) -> Result<()> {
        if to_append.as_ref().is_empty() {
            self.storage.get_file_meta(file_id).await?;
            return Ok(());
        }
        self.storage
            .append_file(file_id, to_append.as_ref())
            .await?;
        self.reindex_file_in_search(file_id).await?;
        self.cache.invalidate(&file_id).await;
        self.bump_doc_wait(file_id);
        Ok(())
    }

    #[instrument(skip(self, new_parent_path, new_name), fields(file_id = file_id.raw()), err(Debug))]
    pub async fn move_document_to_directory(
        &self,
        file_id: EntryId,
        new_parent_path: impl AsRef<str> + Send,
        new_name: impl AsRef<str> + Send,
    ) -> Result<()> {
        self.storage
            .move_file(file_id, new_parent_path.as_ref(), new_name.as_ref())
            .await?;
        self.reindex_file_in_search(file_id).await?;
        self.cache.invalidate(&file_id).await;
        Ok(())
    }

    #[instrument(
        skip(self, new_name),
        fields(file_id = file_id.raw(), new_name = %new_name.as_ref()),
        err(Debug)
    )]
    pub async fn rename_document(
        &self,
        file_id: EntryId,
        new_name: impl AsRef<str> + Send,
    ) -> Result<()> {
        validate_entity_name(new_name.as_ref())?;
        self.storage.rename_file(file_id, new_name.as_ref()).await?;
        self.reindex_file_in_search(file_id).await?;
        self.cache.invalidate(&file_id).await;
        Ok(())
    }

    /// Read-through cache; `touch` runs after content is resolved.
    #[instrument(skip(self), fields(file_id = file_id.raw()), err(Debug))]
    pub async fn get_document(&self, file_id: EntryId) -> Result<String> {
        let content = if self.cache_active {
            if let Some(hit) = self.cache.get(&file_id).await {
                hit
            } else {
                let loaded = self.storage.get_file_content(file_id).await?;
                self.cache.insert(file_id, loaded.clone()).await;
                loaded
            }
        } else {
            self.storage.get_file_content(file_id).await?
        };
        self.storage.touch(file_id).await?;
        Ok(content)
    }

    /// Unix-like touch: `modified_at == None` — create an empty file (with parent dirs) if missing, else bump `modified_at` only.
    /// With `Some(ts)` — set exact `modified_at` on an existing file or directory; if the path is missing, create an empty file then apply `ts`.
    #[instrument(skip(self, path), fields(exact_mtime = modified_at.is_some()), err(Debug))]
    pub async fn touch_document_by_path(
        &self,
        path: impl AsRef<str> + Send,
        modified_at: Option<Timestamp>,
    ) -> Result<()> {
        let path = path.as_ref();
        let segs = canonical_path_segments(path)?;
        match modified_at {
            None => match self.resolve_file_path(path).await {
                Ok(fid) => {
                    self.storage.bump_file_modified_at(fid).await?;
                    self.cache.invalidate(&fid).await;
                    self.bump_doc_wait(fid);
                    Ok(())
                }
                Err(Error::NotFound(_)) => self.put_document_by_path(path, "", false, None).await,
                Err(e) => Err(e),
            },
            Some(ts) => {
                if segs.is_empty() {
                    return Err(Error::InvalidInput("cannot set modified_at on root".into()));
                }
                match self.storage.resolve_path(path, None).await {
                    Ok(id) => {
                        self.storage.set_entry_modified_at(id, ts).await?;
                        if self
                            .storage
                            .resolve_path(path, Some(EntryKind::File))
                            .await
                            .is_ok()
                        {
                            self.cache.invalidate(&id).await;
                        }
                        Ok(())
                    }
                    Err(Error::NotFound(_)) => {
                        self.put_document_by_path(path, "", false, None).await?;
                        let id = self.storage.resolve_path(path, None).await?;
                        self.storage.set_entry_modified_at(id, ts).await?;
                        if self
                            .storage
                            .resolve_path(path, Some(EntryKind::File))
                            .await
                            .is_ok()
                        {
                            self.cache.invalidate(&id).await;
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
        }
    }

    #[instrument(skip(self), fields(dir_path = %dir_path.as_ref()), err(Debug))]
    pub async fn list_directory(
        &self,
        dir_path: impl AsRef<str> + Send,
    ) -> Result<Vec<ListedEntry>> {
        self.storage.list_directory(dir_path.as_ref()).await
    }

    /// Full-text search. `directory_prefix` limits to that directory subtree; `None` or `"/"` searches all.
    #[instrument(
        skip(self, keywords),
        fields(keywords_len = keywords.as_ref().len(), directory = ?directory_prefix),
        err(Debug)
    )]
    pub async fn search(
        &self,
        keywords: impl AsRef<str>,
        directory_prefix: Option<&str>,
    ) -> Result<Vec<EntryId>> {
        let norm = directory_prefix.and_then(|p| {
            let t = p.trim();
            if t.is_empty() || t == "/" {
                None
            } else {
                Some(t.trim_end_matches('/'))
            }
        });
        self.search.search(keywords, norm).await
    }

    #[instrument(skip(self), fields(directory = ?directory_path_filter), err(Debug))]
    pub async fn reindex(&self, directory_path_filter: Option<&str>) -> Result<()> {
        let rows = self
            .storage
            .files_for_search_reindex(directory_path_filter)
            .await?;
        let n = rows.len();
        match directory_path_filter {
            None | Some("" | "/") => self.search.replace_all_from_rows(&rows).await?,
            Some(_) => self.search.upsert_batch(&rows).await?,
        }
        tracing::debug!(
            target: "tabularium::db",
            doc_count = n,
            full = directory_path_filter.is_none()
                || directory_path_filter.is_some_and(|s| s.is_empty() || s == "/"),
            "reindex complete"
        );
        Ok(())
    }
}

#[cfg(test)]
impl<S: Storage> Database<S> {
    pub(crate) async fn test_storage_canonical_path(&self, entry_id: EntryId) -> Result<String> {
        self.storage.canonical_path(entry_id).await
    }

    pub(crate) async fn test_storage_ensure_directory_path(
        &self,
        dir_path: &str,
    ) -> Result<EntryId> {
        self.storage.ensure_directory_path(dir_path).await
    }

    pub(crate) async fn test_storage_resolve_path(
        &self,
        path: &str,
        expected_kind: Option<EntryKind>,
    ) -> Result<EntryId> {
        self.storage.resolve_path(path, expected_kind).await
    }
}

#[cfg(test)]
mod deep_path_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn roundtrip_directory_file_search_and_cache() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("t.db");
        let idx_path = dir.path().join("t.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 16).await.unwrap();
        db.create_directory("/notes", None, false).await.unwrap();
        let id = db
            .create_file_in_directory("/notes", "readme", "alpha beta gamma")
            .await
            .unwrap();
        let body = db.get_document(id).await.unwrap();
        assert_eq!(body, "alpha beta gamma");
        let again = db.get_document(id).await.unwrap();
        assert_eq!(again, body);
        let hits = db.search("beta", None).await.unwrap();
        assert_eq!(hits, vec![id]);
        db.update_document(id, "omega psi").await.unwrap();
        let body2 = db.get_document(id).await.unwrap();
        assert_eq!(body2, "omega psi");
        let hits2 = db.search("beta", None).await.unwrap();
        assert!(hits2.is_empty());
        let hits3 = db.search("omega", None).await.unwrap();
        assert_eq!(hits3, vec![id]);
    }

    #[tokio::test]
    async fn search_indexes_file_name_and_description_reindexes_on_describe() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("sd.db");
        let idx_path = dir.path().join("sd.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 16).await.unwrap();
        db.create_directory("/d", None, false).await.unwrap();
        let id = db
            .create_file_in_directory("/d", "ledger_alpha.md", "zzz boring")
            .await
            .unwrap();
        assert!(db.search("ledger_alpha", None).await.unwrap().contains(&id));
        assert!(db.search("boring", None).await.unwrap().contains(&id));
        assert!(db.search("heretek_marker", None).await.unwrap().is_empty());
        db.set_entry_description("/d/ledger_alpha.md", Some("heretek_marker chronicle"))
            .await
            .unwrap();
        assert!(
            db.search("heretek_marker", None)
                .await
                .unwrap()
                .contains(&id)
        );
        db.set_entry_description("/d/ledger_alpha.md", None)
            .await
            .unwrap();
        assert!(db.search("heretek_marker", None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_directory_with_files_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("e.db");
        let idx_path = dir.path().join("e.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/full", None, false).await.unwrap();
        db.create_file_in_directory("/full", "x", "y")
            .await
            .unwrap();
        let err = db.delete_directory("/full").await.err().unwrap();
        assert!(matches!(err, crate::Error::NotEmpty(_)), "{err:?}");
    }

    #[tokio::test]
    async fn duplicate_file_name_per_directory_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("d.db");
        let idx_path = dir.path().join("d.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/c", None, false).await.unwrap();
        db.create_file_in_directory("/c", "same", "a")
            .await
            .unwrap();
        let err = db
            .create_file_in_directory("/c", "same", "b")
            .await
            .err()
            .unwrap();
        assert!(matches!(err, crate::Error::Duplicate(_)));
    }

    #[tokio::test]
    async fn full_reindex_restores_search_hits() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("r.db");
        let idx_path = dir.path().join("r.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/c", None, false).await.unwrap();
        let id = db
            .create_file_in_directory("/c", "a", "inquisitorial keyword")
            .await
            .unwrap();
        db.reindex(None).await.unwrap();
        let hits = db.search("inquisitorial", None).await.unwrap();
        assert_eq!(hits, vec![id]);
    }

    #[tokio::test]
    async fn list_directory_and_document_size() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("cnt.db");
        let idx_path = dir.path().join("cnt.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/alpha", None, false).await.unwrap();
        db.create_file_in_directory("/alpha", "d1", "hello")
            .await
            .unwrap();
        let rows = db.list_directory("/").await.unwrap();
        let dirs: Vec<_> = rows.iter().filter(|r| r.kind() == EntryKind::Dir).collect();
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs[0].recursive_file_count(), 1);
        let metas: Vec<_> = rows
            .iter()
            .filter(|r| r.kind() == EntryKind::File)
            .collect();
        assert!(metas.is_empty());
        let under = db.list_directory("/alpha").await.unwrap();
        let f = under.iter().find(|e| e.name() == "d1").unwrap();
        assert_eq!(f.size_bytes(), Some(5));
    }

    #[tokio::test]
    async fn subtree_scoped_reindex_keeps_other_directories_searchable() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("s.db");
        let idx_path = dir.path().join("s.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/one", None, false).await.unwrap();
        db.create_directory("/two", None, false).await.unwrap();
        db.create_file_in_directory("/one", "a", "alpha uniqueone")
            .await
            .unwrap();
        let id2 = db
            .create_file_in_directory("/two", "b", "beta uniquetwo")
            .await
            .unwrap();
        db.reindex(None).await.unwrap();
        db.reindex(Some("/one")).await.unwrap();
        let h1 = db.search("uniqueone", None).await.unwrap();
        assert_eq!(h1.len(), 1);
        let h2 = db.search("uniquetwo", None).await.unwrap();
        assert_eq!(h2, vec![id2]);
    }

    #[tokio::test]
    async fn append_document_inserts_single_newline_when_needed() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("nl.db");
        let idx_path = dir.path().join("nl.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/c", None, false).await.unwrap();
        let id = db.create_file_in_directory("/c", "d", "a\n").await.unwrap();
        db.append_document(id, "b").await.unwrap();
        let body = db.get_document(id).await.unwrap();
        assert_eq!(body, "a\nb");
    }

    #[tokio::test]
    async fn append_document_empty_piece_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("empty_append.db");
        let idx_path = dir.path().join("empty_append.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/c", None, false).await.unwrap();
        let id = db.create_file_in_directory("/c", "d", "a").await.unwrap();

        db.append_document(id, "").await.unwrap();

        let body = db.get_document(id).await.unwrap();
        assert_eq!(body, "a");
    }

    #[tokio::test]
    async fn append_document_by_path_creates_missing_document() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("ap.db");
        let idx_path = dir.path().join("ap.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/c", None, false).await.unwrap();
        db.append_document_by_path("/c/newdoc", "hello", false)
            .await
            .unwrap();
        let meta = db.document_ref_by_path("/c/newdoc").await.unwrap();
        let body = db.get_document(meta.id()).await.unwrap();
        assert_eq!(body, "hello");
    }

    /// `force=false` on an already-existing document must error with `Duplicate`,
    /// regardless of whether it's `put_document_by_path` or `append_document_by_path`.
    /// *Ignorantia non excusat* — silent overwrite is heresy.
    #[tokio::test]
    async fn put_and_append_force_false_existing_target_returns_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("force_dup.db");
        let idx_path = dir.path().join("force_dup.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/c", None, false).await.unwrap();

        // Initial create via put(force=false) on missing target succeeds.
        db.put_document_by_path("/c/d", "first", false, None)
            .await
            .unwrap();
        let id = db.document_ref_by_path("/c/d").await.unwrap().id();
        let body = db.get_document(id).await.unwrap();
        assert_eq!(body, "first");

        // put(force=false) on existing → Duplicate (no body change).
        let err = db
            .put_document_by_path("/c/d", "OVERWRITE", false, None)
            .await
            .unwrap_err();
        assert!(matches!(err, crate::Error::Duplicate(_)), "got {err:?}");
        let body = db.get_document(id).await.unwrap();
        assert_eq!(body, "first");

        // append(force=false) on existing → Duplicate (no body change).
        let err = db
            .append_document_by_path("/c/d", "TAIL", false)
            .await
            .unwrap_err();
        assert!(matches!(err, crate::Error::Duplicate(_)), "got {err:?}");
        let body = db.get_document(id).await.unwrap();
        assert_eq!(body, "first");
    }

    /// `force=true` preserves the legacy semantics: put replaces, append appends.
    #[tokio::test]
    async fn put_force_true_replaces_and_append_force_true_appends() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("force_yes.db");
        let idx_path = dir.path().join("force_yes.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/c", None, false).await.unwrap();

        db.put_document_by_path("/c/d", "first", false, None)
            .await
            .unwrap();
        let id = db.document_ref_by_path("/c/d").await.unwrap().id();

        // put(force=true) on existing → overwrite.
        db.put_document_by_path("/c/d", "second", true, None)
            .await
            .unwrap();
        let body = db.get_document(id).await.unwrap();
        assert_eq!(body, "second");

        // append(force=true) on existing → append (single newline boundary).
        db.append_document_by_path("/c/d", "tail", true)
            .await
            .unwrap();
        let body = db.get_document(id).await.unwrap();
        assert_eq!(body, "second\ntail");
    }

    /// Concurrent `force=false` callers racing on the same fresh path must produce
    /// exactly one `Ok` and one `Duplicate`. This proves atomicity at the storage
    /// layer (no exists-then-create TOCTOU window). The Emperor protects.
    #[tokio::test]
    async fn put_force_false_concurrent_creators_atomic_one_winner() {
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("race.db");
        let idx_path = dir.path().join("race.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = Arc::new(SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap());
        db.create_directory("/c", None, false).await.unwrap();

        let path = "/c/contested";
        let n: usize = 8;
        let mut handles = Vec::with_capacity(n);
        for i in 0..n {
            let db = Arc::clone(&db);
            let payload = format!("agent-{i}");
            handles.push(tokio::spawn(async move {
                db.put_document_by_path(path, &payload, false, None).await
            }));
        }

        let mut ok_count = 0;
        let mut dup_count = 0;
        let mut other = Vec::new();
        for h in handles {
            match h.await.unwrap() {
                Ok(()) => ok_count += 1,
                Err(crate::Error::Duplicate(_)) => dup_count += 1,
                Err(e) => other.push(e),
            }
        }
        assert!(other.is_empty(), "unexpected error variants: {other:?}");
        assert_eq!(ok_count, 1, "exactly one creator must win the race");
        assert_eq!(dup_count, n - 1, "all losers must see Duplicate");

        // Document exists and contains exactly one of the candidate payloads.
        let body = db
            .get_document(db.document_ref_by_path(path).await.unwrap().id())
            .await
            .unwrap();
        assert!(body.starts_with("agent-"), "got body {body:?}");
    }

    #[tokio::test]
    async fn say_document_by_path_formats_markdown_block_and_trims_trailing_newlines() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("say.db");
        let idx_path = dir.path().join("say.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/c", None, false).await.unwrap();
        let id = db.create_file_in_directory("/c", "d", "x\n").await.unwrap();
        db.say_document_by_path("/c/d", "ada", "hello\n")
            .await
            .unwrap();
        let body = db.get_document(id).await.unwrap();
        assert_eq!(body, "x\n\n## ada\n\nhello\n\n");
    }

    #[tokio::test]
    async fn say_document_by_path_no_extra_blank_when_body_already_has_paragraph_break() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("say_gap.db");
        let idx_path = dir.path().join("say_gap.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/c", None, false).await.unwrap();
        let id = db
            .create_file_in_directory("/c", "d", "x\n\n")
            .await
            .unwrap();
        db.say_document_by_path("/c/d", "ada", "hello")
            .await
            .unwrap();
        let body = db.get_document(id).await.unwrap();
        assert_eq!(body, "x\n\n## ada\n\nhello\n\n");
    }

    #[tokio::test]
    async fn say_document_by_path_rejects_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("say_miss.db");
        let idx_path = dir.path().join("say_miss.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/c", None, false).await.unwrap();
        let err = db
            .say_document_by_path("/c/nope", "ada", "x")
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("say_document") && msg.contains("does not exist"),
            "{msg}"
        );
    }

    #[tokio::test]
    async fn touch_document_by_path_creates_and_bumps_modified() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("touch.db");
        let idx_path = dir.path().join("touch.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/t", None, false).await.unwrap();
        db.touch_document_by_path("/t/new", None).await.unwrap();
        let m0 = db.document_ref_by_path("/t/new").await.unwrap();
        assert_eq!(db.get_document(m0.id()).await.unwrap(), "");
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        db.touch_document_by_path("/t/new", None).await.unwrap();
        let m1 = db.document_ref_by_path("/t/new").await.unwrap();
        assert!(m1.modified_at() > m0.modified_at());
        assert_eq!(m1.created_at(), m0.created_at());
    }

    #[tokio::test]
    async fn touch_document_by_path_with_ts_sets_file_and_directory_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("smtime.db");
        let idx_path = dir.path().join("smtime.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/m", None, false).await.unwrap();
        db.create_file_in_directory("/m", "f", "x").await.unwrap();
        let ts = Timestamp::from_secs(1_700_000_000);
        db.touch_document_by_path("/m/f", Some(ts)).await.unwrap();
        let mf = db.document_ref_by_path("/m/f").await.unwrap();
        assert_eq!(mf.modified_at(), ts);
        db.touch_document_by_path("/m", Some(ts)).await.unwrap();
        let rows = db.list_directory("/").await.unwrap();
        let row = rows.iter().find(|r| r.name() == "m").unwrap();
        assert_eq!(row.modified_at(), ts);
    }

    #[tokio::test]
    async fn touch_document_by_path_with_ts_creates_empty_at_exact_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("touch_ts.db");
        let idx_path = dir.path().join("touch_ts.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/tc", None, false).await.unwrap();
        let ts = Timestamp::from_secs(1_710_000_000);
        db.touch_document_by_path("/tc/pinned", Some(ts))
            .await
            .unwrap();
        let m = db.document_ref_by_path("/tc/pinned").await.unwrap();
        assert_eq!(m.modified_at(), ts);
        assert_eq!(db.get_document(m.id()).await.unwrap(), "");
    }

    #[tokio::test]
    async fn delete_directory_recursive_removes_documents() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("rec.db");
        let idx_path = dir.path().join("rec.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/big", None, false).await.unwrap();
        let id = db
            .create_file_in_directory("/big", "x", "needle_recursive")
            .await
            .unwrap();
        db.delete_directory_recursive("/big").await.unwrap();
        assert!(db.get_document(id).await.is_err());
        assert!(db.resolve_directory_path("/big").await.is_err());
    }

    #[tokio::test]
    async fn append_if_not_contains_appends_when_marker_absent() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("ainc.db");
        let idx_path = dir.path().join("ainc.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/c", None, false).await.unwrap();
        db.create_file_in_directory("/c", "d", "alpha")
            .await
            .unwrap();
        assert!(
            db.append_if_not_contains_by_path("/c/d", "OMEGA", "\nOMEGA\n")
                .await
                .unwrap()
        );
        assert!(
            !db.append_if_not_contains_by_path("/c/d", "OMEGA", "\nOMEGA\n")
                .await
                .unwrap()
        );
        let id = db.resolve_file_path("/c/d").await.unwrap();
        let body = db.get_document(id).await.unwrap();
        assert_eq!(body.matches("OMEGA").count(), 1);
    }

    #[tokio::test]
    async fn append_if_not_contains_substring_sees_done_inside_undone() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("substr.db");
        let idx_path = dir.path().join("substr.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/c", None, false).await.unwrap();
        db.create_file_in_directory("/c", "d", "UNDONE")
            .await
            .unwrap();
        assert!(
            !db.append_if_not_contains_by_path("/c/d", "DONE", "x")
                .await
                .unwrap()
        );
        let id = db.resolve_file_path("/c/d").await.unwrap();
        assert_eq!(db.get_document(id).await.unwrap(), "UNDONE");
    }

    #[tokio::test]
    async fn append_if_not_contains_rejects_empty_marker() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("mk.db");
        let idx_path = dir.path().join("mk.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/c", None, false).await.unwrap();
        db.create_file_in_directory("/c", "d", "x").await.unwrap();
        let e = db
            .append_if_not_contains_by_path("/c/d", "", "y")
            .await
            .unwrap_err();
        assert!(e.to_string().contains("marker must be non-empty"), "{e}");
    }

    #[tokio::test]
    async fn append_if_not_contains_errors_when_document_missing() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("miss.db");
        let idx_path = dir.path().join("miss.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/c", None, false).await.unwrap();
        assert!(matches!(
            db.append_if_not_contains_by_path("/c/nope", "M", "z").await,
            Err(crate::Error::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn append_if_not_contains_concurrent_only_one_inserts_marker() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("conc.db");
        let idx_path = dir.path().join("conc.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = Arc::new(SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap());
        db.create_directory("/c", None, false).await.unwrap();
        db.create_file_in_directory("/c", "d", "start")
            .await
            .unwrap();
        let marker = "<<<FLAG>>>";
        let piece = format!("\n{marker}\n");
        let n = 48usize;
        let mut handles = Vec::with_capacity(n);
        for _ in 0..n {
            let db = db.clone();
            let p = piece.clone();
            handles.push(tokio::spawn(async move {
                db.append_if_not_contains_by_path("/c/d", marker, &p)
                    .await
                    .unwrap()
            }));
        }
        let mut truths = 0usize;
        for h in handles {
            if h.await.unwrap() {
                truths += 1;
            }
        }
        assert_eq!(truths, 1);
        let id = db.resolve_file_path("/c/d").await.unwrap();
        let body = db.get_document(id).await.unwrap();
        assert_eq!(body.matches(marker).count(), 1);
    }

    #[tokio::test]
    async fn file_revision_stable_on_read_advances_on_write() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("rev.db");
        let idx_path = dir.path().join("rev.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap();
        db.create_directory("/rev", None, false).await.unwrap();
        let id = db
            .create_document_at_path("/rev/f", "body", false, None)
            .await
            .unwrap();
        let rev0 = db
            .document_ref_by_path("/rev/f")
            .await
            .unwrap()
            .revision()
            .expect("revision")
            .to_string();
        db.get_document(id).await.unwrap();
        let meta_read = db.document_ref_by_path("/rev/f").await.unwrap();
        let rev_after_read = meta_read.revision().unwrap();
        assert_eq!(rev_after_read, rev0.as_str());
        db.update_document(id, "next").await.unwrap();
        let meta2 = db.document_ref_by_path("/rev/f").await.unwrap();
        let rev2 = meta2.revision().unwrap();
        assert_ne!(rev2, rev0.as_str());
    }

    #[tokio::test]
    async fn wait_document_fires_on_update() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("w.db");
        let idx_path = dir.path().join("w.idx");
        let uri = format!("sqlite://{}", db_path.display());
        let db = Arc::new(SqliteDatabase::init(&uri, &idx_path, 0).await.unwrap());
        db.create_directory("/w", None, false).await.unwrap();
        let id = db.create_file_in_directory("/w", "d", "v0").await.unwrap();
        let db_wait = db.clone();
        let j = tokio::spawn(async move {
            db_wait
                .wait_until_document_changed(id, Duration::from_secs(5))
                .await
        });
        tokio::time::sleep(Duration::from_millis(40)).await;
        db.update_document(id, "v1").await.unwrap();
        let st = j.await.unwrap().unwrap();
        assert_eq!(st, DocumentWaitStatus::Changed);
    }
}

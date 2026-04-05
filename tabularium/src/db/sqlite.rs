//! SQLite-backed `Storage` — the first forge of the Tabularium.

use async_trait::async_trait;
use bma_ts::Timestamp;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow};
use sqlx::{QueryBuilder, Row, SqlitePool};
use tracing::instrument;

use super::EntryId;
use super::entry_kind::EntryKind;
use super::meta::{DocumentMeta, ListedEntry};
use super::storage::Storage;
use crate::resource_path::{canonical_path_segments, parent_and_final_name};
use crate::{Error, Result};

const ROOT_ID: i64 = 1;

async fn table_exists(pool: &SqlitePool, name: &str) -> Result<bool> {
    let n: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?")
            .bind(name)
            .fetch_one(pool)
            .await?;
    Ok(n > 0)
}

async fn migrate(pool: &SqlitePool) -> Result<()> {
    if table_exists(pool, "entries").await? {
        return Ok(());
    }
    sqlx::query(
        r"CREATE TABLE entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    parent_id INTEGER NOT NULL REFERENCES entries(id) ON DELETE RESTRICT,
    kind INTEGER NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    mime_type TEXT DEFAULT NULL,
    content TEXT,
    size INTEGER,
    created_at INTEGER NOT NULL,
    modified_at INTEGER NOT NULL,
    accessed_at INTEGER NOT NULL,
    UNIQUE(parent_id, name),
    CHECK(parent_id != id OR id = 1),
    CHECK((kind = 0 AND content IS NULL AND size IS NULL) OR (kind = 1 AND content IS NOT NULL AND size IS NOT NULL))
)",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_entries_parent ON entries(parent_id)")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_entries_kind ON entries(kind)")
        .execute(pool)
        .await?;
    let now = Timestamp::now();
    sqlx::query(
        r"INSERT INTO entries (id, parent_id, kind, name, description, content, size, created_at, modified_at, accessed_at)
         VALUES (1, 1, 0, '', NULL, NULL, NULL, ?, ?, ?)",
    )
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// SQLite implementation of [`Storage`].
pub struct SqliteStorage {
    pool: SqlitePool,
}

impl SqliteStorage {
    /// Connects (creating the database file if needed) and applies DDL.
    #[instrument(name = "sqlite_storage_connect", fields(db_uri = db_uri.as_ref()), err(Debug))]
    pub async fn connect(db_uri: impl AsRef<str>) -> Result<Self> {
        let opts = db_uri
            .as_ref()
            .parse::<SqliteConnectOptions>()
            .map_err(|e| Error::InvalidInput(e.to_string()))?
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(opts)
            .await?;
        migrate(&pool).await?;
        Ok(Self { pool })
    }

    fn map_sqlite_constraint(err: sqlx::Error) -> Error {
        if let Some(db) = err.as_database_error() {
            let msg = db.message();
            if msg.contains("UNIQUE") {
                return Error::Duplicate("name already exists in directory".into());
            }
            if msg.contains("FOREIGN KEY") || msg.contains("RESTRICT") {
                return Error::InvalidInput("reference conflict".into());
            }
        }
        err.into()
    }

    fn map_dir_delete_fk(err: sqlx::Error) -> Error {
        if let Some(db) = err.as_database_error()
            && (db.message().contains("FOREIGN KEY") || db.message().contains("RESTRICT"))
        {
            return Error::NotEmpty("directory is not empty".into());
        }
        err.into()
    }
}

/// JSON array of path segments for [`WITH RECURSIVE`] resolution (Ferrum gate: one CTE walk).
pub(crate) fn path_segments_json(segments: &[String]) -> Result<String> {
    serde_json::to_string(segments).map_err(|e| Error::InvalidInput(e.to_string()))
}

async fn resolve_path_id(
    pool: &SqlitePool,
    segments: &[String],
    expected_kind: Option<EntryKind>,
) -> Result<EntryId> {
    if segments.is_empty() {
        let id = EntryId::from_raw(ROOT_ID);
        if let Some(k) = expected_kind {
            let row_kind: EntryKind = sqlx::query_scalar("SELECT kind FROM entries WHERE id = ?")
                .bind(ROOT_ID)
                .fetch_one(pool)
                .await?;
            if row_kind != k {
                return Err(Error::NotFound("entry /".into()));
            }
        }
        return Ok(id);
    }
    let json = path_segments_json(segments)?;
    let id: Option<i64> = sqlx::query_scalar(
        r"
        WITH RECURSIVE
        segs(idx, seg_name) AS (
            SELECT CAST(key AS INTEGER), value FROM json_each(?1)
        ),
        walk(cur_id, ri) AS (
            SELECT 1, -1
            UNION ALL
            SELECT e.id, s.idx
            FROM walk w
            JOIN segs s ON s.idx = w.ri + 1
            JOIN entries e ON e.parent_id = w.cur_id AND e.name = s.seg_name
        )
        SELECT cur_id FROM walk WHERE ri = (SELECT MAX(idx) FROM segs)
        ",
    )
    .bind(&json)
    .fetch_optional(pool)
    .await?;
    let Some(raw) = id else {
        let p = format!("/{}", segments.join("/"));
        return Err(Error::NotFound(p));
    };
    if let Some(k) = expected_kind {
        let row_kind: EntryKind = sqlx::query_scalar("SELECT kind FROM entries WHERE id = ?")
            .bind(raw)
            .fetch_one(pool)
            .await?;
        if row_kind != k {
            let p = format!("/{}", segments.join("/"));
            return Err(Error::NotFound(p));
        }
    }
    Ok(EntryId::from_raw(raw))
}

async fn canonical_path_for_id(pool: &SqlitePool, entry_id: EntryId) -> Result<String> {
    if entry_id.raw() == ROOT_ID {
        return Ok("/".to_string());
    }
    let names: Vec<String> = sqlx::query_scalar(
        r"
        WITH RECURSIVE up AS (
            SELECT id, parent_id, name, 0 AS lvl FROM entries WHERE id = ?
            UNION ALL
            SELECT e.id, e.parent_id, e.name, up.lvl + 1
            FROM entries e
            INNER JOIN up ON e.id = up.parent_id
            WHERE up.parent_id != up.id
        )
        SELECT name FROM up ORDER BY lvl ASC
        ",
    )
    .bind(entry_id.raw())
    .fetch_all(pool)
    .await?;
    if names.is_empty() {
        return Err(Error::NotFound(format!("entry {}", entry_id.raw())));
    }
    let mut parts: Vec<&str> = names.iter().map(String::as_str).collect();
    parts.reverse();
    if parts.first().is_some_and(|s| s.is_empty()) {
        parts.remove(0);
    }
    if parts.is_empty() {
        return Ok("/".to_string());
    }
    Ok(format!("/{}", parts.join("/")))
}

async fn subtree_ids_deepest_first(pool: &SqlitePool, root_id: i64) -> Result<Vec<i64>> {
    sqlx::query_scalar(
        r"
        WITH RECURSIVE sub AS (
            SELECT id, 0 AS depth FROM entries WHERE id = ?
            UNION ALL
            SELECT e.id, sub.depth + 1
            FROM entries e
            JOIN sub ON e.parent_id = sub.id
            WHERE e.id != e.parent_id
        )
        SELECT id FROM sub ORDER BY depth DESC
        ",
    )
    .bind(root_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

async fn recursive_file_count_under(pool: &SqlitePool, dir_id: i64) -> Result<u64> {
    let n: i64 = sqlx::query_scalar(
        r"
        WITH RECURSIVE sub AS (
            SELECT id FROM entries WHERE id = ?
            UNION ALL
            SELECT e.id FROM entries e
            JOIN sub ON e.parent_id = sub.id
            WHERE e.id != e.parent_id
        )
        SELECT COUNT(*) FROM entries WHERE kind = 1 AND id IN (SELECT id FROM sub)
        ",
    )
    .bind(dir_id)
    .fetch_one(pool)
    .await?;
    u64::try_from(n).map_err(|_| Error::InvalidInput("file count overflow".into()))
}

fn row_to_file_meta(row: &SqliteRow, canonical_path: String) -> Result<DocumentMeta> {
    Ok(DocumentMeta::new(
        EntryId::from_raw(row.try_get::<i64, _>(0)?),
        EntryId::from_raw(row.try_get::<i64, _>(1)?),
        row.try_get::<String, _>(2)?,
        canonical_path,
        row.try_get::<Timestamp, _>(3)?,
        row.try_get::<Timestamp, _>(4)?,
        row.try_get::<Timestamp, _>(5)?,
        row.try_get::<i64, _>(6)?,
    ))
}

#[async_trait]
impl Storage for SqliteStorage {
    #[instrument(skip(self), fields(file_id = file_id.raw()), err(Debug))]
    async fn touch(&self, file_id: EntryId) -> Result<()> {
        let now = Timestamp::now();
        let r = sqlx::query("UPDATE entries SET accessed_at = ? WHERE id = ? AND kind = 1")
            .bind(now)
            .bind(file_id.raw())
            .execute(&self.pool)
            .await?;
        if r.rows_affected() == 0 {
            return Err(Error::NotFound(format!("file {}", file_id.raw())));
        }
        Ok(())
    }

    /// `NotFound` is normal when probing file-vs-directory (REST `get_or_list`); omit `err(Debug)` to avoid ERROR spam.
    #[instrument(skip(self), fields(path = %path, expected_kind = ?expected_kind))]
    async fn resolve_path(&self, path: &str, expected_kind: Option<EntryKind>) -> Result<EntryId> {
        let segs = canonical_path_segments(path)?;
        resolve_path_id(&self.pool, &segs, expected_kind).await
    }

    #[instrument(skip(self), fields(entry_id = entry_id.raw()), err(Debug))]
    async fn canonical_path(&self, entry_id: EntryId) -> Result<String> {
        canonical_path_for_id(&self.pool, entry_id).await
    }

    #[instrument(skip(self), fields(file_id = file_id.raw()), err(Debug))]
    async fn parent_dir_path_for_file(&self, file_id: EntryId) -> Result<String> {
        let parent: i64 =
            sqlx::query_scalar("SELECT parent_id FROM entries WHERE id = ? AND kind = 1")
                .bind(file_id.raw())
                .fetch_optional(&self.pool)
                .await?
                .ok_or_else(|| Error::NotFound(format!("file {}", file_id.raw())))?;
        canonical_path_for_id(&self.pool, EntryId::from_raw(parent)).await
    }

    #[instrument(skip(self, description), fields(path = %path), err(Debug))]
    async fn create_directory(&self, path: &str, description: Option<&str>) -> Result<EntryId> {
        let (parent_path, name) = parent_and_final_name(path)?;
        let parent_id = self
            .resolve_path(&parent_path, Some(EntryKind::Dir))
            .await?;
        let now = Timestamp::now();
        let id: i64 = sqlx::query_scalar(
            r"INSERT INTO entries (parent_id, kind, name, description, content, size, created_at, modified_at, accessed_at)
            VALUES (?, 0, ?, ?, NULL, NULL, ?, ?, ?) RETURNING id",
        )
        .bind(parent_id.raw())
        .bind(&name)
        .bind(description)
        .bind(now)
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(Self::map_sqlite_constraint)?;
        Ok(EntryId::from_raw(id))
    }

    #[instrument(skip(self), fields(path = %path), err(Debug))]
    async fn delete_directory(&self, path: &str) -> Result<()> {
        let id = self.resolve_path(path, Some(EntryKind::Dir)).await?;
        if id.raw() == ROOT_ID {
            return Err(Error::InvalidInput("cannot delete root".into()));
        }
        let r = sqlx::query("DELETE FROM entries WHERE id = ? AND kind = 0")
            .bind(id.raw())
            .execute(&self.pool)
            .await
            .map_err(Self::map_dir_delete_fk)?;
        if r.rows_affected() == 0 {
            return Err(Error::NotFound(path.to_string()));
        }
        Ok(())
    }

    #[instrument(skip(self), fields(path = %path), err(Debug))]
    async fn delete_directory_recursive(&self, path: &str) -> Result<()> {
        let id = self.resolve_path(path, Some(EntryKind::Dir)).await?;
        if id.raw() == ROOT_ID {
            return Err(Error::InvalidInput("cannot delete root".into()));
        }
        let ids = subtree_ids_deepest_first(&self.pool, id.raw()).await?;
        let mut tx = self.pool.begin().await?;
        for eid in ids {
            sqlx::query("DELETE FROM entries WHERE id = ?")
                .bind(eid)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    #[instrument(skip(self), fields(old_path = %old_path, new_path = %new_path), err(Debug))]
    async fn rename_directory(&self, old_path: &str, new_path: &str) -> Result<()> {
        let (p_old, name_old) = parent_and_final_name(old_path)?;
        let (p_new, name_new) = parent_and_final_name(new_path)?;
        if p_old != p_new {
            return Err(Error::InvalidInput(
                "rename_directory: parent must match (use move for relocation)".into(),
            ));
        }
        let id = self.resolve_path(old_path, Some(EntryKind::Dir)).await?;
        if id.raw() == ROOT_ID {
            return Err(Error::InvalidInput("cannot rename root".into()));
        }
        let now = Timestamp::now();
        let r =
            sqlx::query("UPDATE entries SET name = ?, modified_at = ? WHERE id = ? AND kind = 0")
                .bind(&name_new)
                .bind(now)
                .bind(id.raw())
                .execute(&self.pool)
                .await
                .map_err(Self::map_sqlite_constraint)?;
        if r.rows_affected() == 0 {
            return Err(Error::NotFound(old_path.to_string()));
        }
        let _ = name_old;
        Ok(())
    }

    #[instrument(skip(self, content), fields(path = %path), err(Debug))]
    async fn create_file(&self, path: &str, content: impl AsRef<str> + Send) -> Result<EntryId> {
        let (parent_path, name) = parent_and_final_name(path)?;
        let parent_id = self
            .resolve_path(&parent_path, Some(EntryKind::Dir))
            .await?;
        let now = Timestamp::now();
        let size = i64::try_from(content.as_ref().len())
            .map_err(|_| Error::InvalidInput("file content size overflow".into()))?;
        let id: i64 = sqlx::query_scalar(
            r"INSERT INTO entries (parent_id, kind, name, description, content, size, created_at, modified_at, accessed_at)
            VALUES (?, 1, ?, NULL, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(parent_id.raw())
        .bind(&name)
        .bind(content.as_ref())
        .bind(size)
        .bind(now)
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(Self::map_sqlite_constraint)?;
        Ok(EntryId::from_raw(id))
    }

    #[instrument(skip(self), fields(file_id = file_id.raw()), err(Debug))]
    async fn delete_file(&self, file_id: EntryId) -> Result<()> {
        let r = sqlx::query("DELETE FROM entries WHERE id = ? AND kind = 1")
            .bind(file_id.raw())
            .execute(&self.pool)
            .await?;
        if r.rows_affected() == 0 {
            return Err(Error::NotFound(format!("file {}", file_id.raw())));
        }
        Ok(())
    }

    #[instrument(skip(self, new_content), fields(file_id = file_id.raw()), err(Debug))]
    async fn update_file(
        &self,
        file_id: EntryId,
        new_content: impl AsRef<str> + Send,
    ) -> Result<()> {
        let now = Timestamp::now();
        let size = i64::try_from(new_content.as_ref().len())
            .map_err(|_| Error::InvalidInput("file content size overflow".into()))?;
        let r = sqlx::query(
            "UPDATE entries SET content = ?, size = ?, modified_at = ? WHERE id = ? AND kind = 1",
        )
        .bind(new_content.as_ref())
        .bind(size)
        .bind(now)
        .bind(file_id.raw())
        .execute(&self.pool)
        .await?;
        if r.rows_affected() == 0 {
            return Err(Error::NotFound(format!("file {}", file_id.raw())));
        }
        Ok(())
    }

    #[instrument(skip(self, to_append), fields(file_id = file_id.raw()), err(Debug))]
    async fn append_file(&self, file_id: EntryId, to_append: impl AsRef<str> + Send) -> Result<()> {
        let now = Timestamp::now();
        let piece = to_append.as_ref();
        let r = sqlx::query(
            r"UPDATE entries SET
                content = content || CASE WHEN substr(content, -1) = char(10) THEN '' ELSE char(10) END || ?,
                modified_at = ?,
                size = length(content || CASE WHEN substr(content, -1) = char(10) THEN '' ELSE char(10) END || ?)
            WHERE id = ? AND kind = 1",
        )
        .bind(piece)
        .bind(now)
        .bind(piece)
        .bind(file_id.raw())
        .execute(&self.pool)
        .await?;
        if r.rows_affected() == 0 {
            return Err(Error::NotFound(format!("file {}", file_id.raw())));
        }
        Ok(())
    }

    #[instrument(skip(self), fields(file_id = file_id.raw()), err(Debug))]
    async fn bump_file_modified_at(&self, file_id: EntryId) -> Result<()> {
        let now = Timestamp::now();
        let r = sqlx::query("UPDATE entries SET modified_at = ? WHERE id = ? AND kind = 1")
            .bind(now)
            .bind(file_id.raw())
            .execute(&self.pool)
            .await?;
        if r.rows_affected() == 0 {
            return Err(Error::NotFound(format!("file {}", file_id.raw())));
        }
        Ok(())
    }

    #[instrument(skip(self), fields(entry_id = entry_id.raw()), err(Debug))]
    async fn set_entry_modified_at(&self, entry_id: EntryId, modified_at: Timestamp) -> Result<()> {
        if entry_id.raw() == ROOT_ID {
            return Err(Error::InvalidInput("cannot set modified_at on root".into()));
        }
        let r = sqlx::query("UPDATE entries SET modified_at = ? WHERE id = ?")
            .bind(modified_at)
            .bind(entry_id.raw())
            .execute(&self.pool)
            .await?;
        if r.rows_affected() == 0 {
            return Err(Error::NotFound(format!("entry {}", entry_id.raw())));
        }
        Ok(())
    }

    #[instrument(
        skip(self),
        fields(file_id = file_id.raw(), new_parent_path = %new_parent_path, new_name = %new_name),
        err(Debug)
    )]
    async fn move_file(
        &self,
        file_id: EntryId,
        new_parent_path: &str,
        new_name: &str,
    ) -> Result<()> {
        let new_parent = self
            .resolve_path(new_parent_path, Some(EntryKind::Dir))
            .await?;
        let now = Timestamp::now();
        let r = sqlx::query(
            "UPDATE entries SET parent_id = ?, name = ?, modified_at = ? WHERE id = ? AND kind = 1",
        )
        .bind(new_parent.raw())
        .bind(new_name)
        .bind(now)
        .bind(file_id.raw())
        .execute(&self.pool)
        .await;
        match r {
            Ok(r) if r.rows_affected() > 0 => Ok(()),
            Ok(_) => Err(Error::NotFound(format!("file {}", file_id.raw()))),
            Err(e) => Err(Self::map_sqlite_constraint(e)),
        }
    }

    #[instrument(skip(self, new_name), fields(file_id = file_id.raw()), err(Debug))]
    async fn rename_file(&self, file_id: EntryId, new_name: impl AsRef<str> + Send) -> Result<()> {
        let now = Timestamp::now();
        let r =
            sqlx::query("UPDATE entries SET name = ?, modified_at = ? WHERE id = ? AND kind = 1")
                .bind(new_name.as_ref())
                .bind(now)
                .bind(file_id.raw())
                .execute(&self.pool)
                .await;
        match r {
            Ok(r) if r.rows_affected() > 0 => Ok(()),
            Ok(_) => Err(Error::NotFound(format!("file {}", file_id.raw()))),
            Err(e) => Err(Self::map_sqlite_constraint(e)),
        }
    }

    #[instrument(skip(self), fields(file_id = file_id.raw()), err(Debug))]
    async fn get_file_content(&self, file_id: EntryId) -> Result<String> {
        let row = sqlx::query_scalar::<_, String>(
            "SELECT content FROM entries WHERE id = ? AND kind = 1",
        )
        .bind(file_id.raw())
        .fetch_optional(&self.pool)
        .await?;
        row.ok_or_else(|| Error::NotFound(format!("file {}", file_id.raw())))
    }

    #[instrument(skip(self), fields(dir_path = %dir_path), err(Debug))]
    async fn list_directory(&self, dir_path: &str) -> Result<Vec<ListedEntry>> {
        let dir_id = self.resolve_path(dir_path, Some(EntryKind::Dir)).await?;
        let rows = sqlx::query(
            r"SELECT id, kind, name, description, created_at, modified_at, accessed_at, size
            FROM entries WHERE parent_id = ? AND id != parent_id ORDER BY kind, name",
        )
        .bind(dir_id.raw())
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: i64 = row.try_get(0)?;
            let kind: EntryKind = row.try_get(1)?;
            let name: String = row.try_get(2)?;
            let description: Option<String> = row.try_get(3)?;
            let created_at: Timestamp = row.try_get(4)?;
            let modified_at: Timestamp = row.try_get(5)?;
            let accessed_at: Timestamp = row.try_get(6)?;
            let size: Option<i64> = row.try_get(7)?;
            let recursive_file_count = if kind == EntryKind::Dir {
                recursive_file_count_under(&self.pool, id).await?
            } else {
                0
            };
            out.push(ListedEntry::new(
                EntryId::from_raw(id),
                kind,
                name,
                description,
                created_at,
                modified_at,
                accessed_at,
                if kind == EntryKind::File { size } else { None },
                recursive_file_count,
            ));
        }
        Ok(out)
    }

    #[instrument(skip(self), fields(file_id = file_id.raw()), err(Debug))]
    async fn get_file_meta(&self, file_id: EntryId) -> Result<DocumentMeta> {
        let row = sqlx::query(
            r"SELECT id, parent_id, name, created_at, modified_at, accessed_at, size
            FROM entries WHERE id = ? AND kind = 1",
        )
        .bind(file_id.raw())
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Err(Error::NotFound(format!("file {}", file_id.raw())));
        };
        let cp = canonical_path_for_id(&self.pool, file_id).await?;
        row_to_file_meta(&row, cp)
    }

    #[instrument(skip(self, ids), fields(hit_count = ids.len()), err(Debug))]
    async fn files_display_batch(&self, ids: &[EntryId]) -> Result<Vec<(EntryId, String, String)>> {
        const CHUNK: usize = 500;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(CHUNK) {
            let mut qb = QueryBuilder::new("SELECT id FROM entries WHERE kind = 1 AND id IN (");
            {
                let mut sep = qb.separated(", ");
                for id in chunk {
                    sep.push_bind(id.raw());
                }
            }
            qb.push(')');
            let rows = qb.build().fetch_all(&self.pool).await?;
            for row in rows {
                let id = EntryId::from_raw(row.try_get::<i64, _>(0)?);
                let path = canonical_path_for_id(&self.pool, id).await?;
                let content: String =
                    sqlx::query_scalar("SELECT content FROM entries WHERE id = ?")
                        .bind(id.raw())
                        .fetch_one(&self.pool)
                        .await?;
                out.push((id, path, content));
            }
        }
        Ok(out)
    }

    #[instrument(skip(self), fields(file_id = file_id.raw()), err(Debug))]
    async fn file_parent_and_content(&self, file_id: EntryId) -> Result<(EntryId, String)> {
        let row = sqlx::query("SELECT parent_id, content FROM entries WHERE id = ? AND kind = 1")
            .bind(file_id.raw())
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else {
            return Err(Error::NotFound(format!("file {}", file_id.raw())));
        };
        let pid: i64 = row.try_get(0)?;
        let content: String = row.try_get(1)?;
        Ok((EntryId::from_raw(pid), content))
    }

    #[instrument(skip(self), fields(filter = ?directory_path_filter), err(Debug))]
    async fn files_for_search_reindex(
        &self,
        directory_path_filter: Option<&str>,
    ) -> Result<Vec<(EntryId, String, String, String, String)>> {
        let file_ids: Vec<i64> = match directory_path_filter {
            None => {
                sqlx::query_scalar("SELECT id FROM entries WHERE kind = 1")
                    .fetch_all(&self.pool)
                    .await?
            }
            Some(p) => {
                let dir_id = self.resolve_path(p, Some(EntryKind::Dir)).await?;
                sqlx::query_scalar(
                    r"
                    WITH RECURSIVE sub AS (
                        SELECT id FROM entries WHERE id = ?
                        UNION ALL
                        SELECT e.id FROM entries e
                        JOIN sub ON e.parent_id = sub.id
                        WHERE e.id != e.parent_id
                    )
                    SELECT id FROM entries WHERE kind = 1 AND id IN (SELECT id FROM sub)
                    ",
                )
                .bind(dir_id.raw())
                .fetch_all(&self.pool)
                .await?
            }
        };
        let mut out = Vec::with_capacity(file_ids.len());
        for raw in file_ids {
            let id = EntryId::from_raw(raw);
            let parent = self.parent_dir_path_for_file(id).await?;
            let row = sqlx::query(
                "SELECT name, description, content FROM entries WHERE id = ? AND kind = 1",
            )
            .bind(raw)
            .fetch_one(&self.pool)
            .await?;
            let name: String = row.try_get(0)?;
            let description: Option<String> = row.try_get(1)?;
            let content: String = row.try_get(2)?;
            out.push((id, parent, name, description.unwrap_or_default(), content));
        }
        Ok(out)
    }

    async fn file_search_index_fields(
        &self,
        file_id: EntryId,
    ) -> Result<(String, String, String, String)> {
        let dp = self.parent_dir_path_for_file(file_id).await?;
        let row =
            sqlx::query("SELECT name, description, content FROM entries WHERE id = ? AND kind = 1")
                .bind(file_id.raw())
                .fetch_optional(&self.pool)
                .await?;
        let Some(row) = row else {
            return Err(Error::NotFound(format!("file {}", file_id.raw())));
        };
        let name: String = row.try_get(0)?;
        let description: Option<String> = row.try_get(1)?;
        let content: String = row.try_get(2)?;
        Ok((dp, name, description.unwrap_or_default(), content))
    }

    #[instrument(
        skip(self),
        fields(dir_id = dir_id.raw(), new_parent_path = %new_parent_path, new_name = %new_name),
        err(Debug)
    )]
    async fn move_directory(
        &self,
        dir_id: EntryId,
        new_parent_path: &str,
        new_name: &str,
    ) -> Result<()> {
        if dir_id.raw() == ROOT_ID {
            return Err(Error::InvalidInput("cannot move root".into()));
        }
        let new_parent = self
            .resolve_path(new_parent_path, Some(EntryKind::Dir))
            .await?;
        let mut tx = self.pool.begin().await?;
        let cycle: i64 = sqlx::query_scalar(
            r"
            WITH RECURSIVE sub AS (
                SELECT id FROM entries WHERE id = ?
                UNION ALL
                SELECT e.id FROM entries e
                JOIN sub ON e.parent_id = sub.id
                WHERE e.id != e.parent_id
            )
            SELECT COUNT(*) FROM sub WHERE id = ?
            ",
        )
        .bind(dir_id.raw())
        .bind(new_parent.raw())
        .fetch_one(&mut *tx)
        .await?;
        if cycle > 0 {
            return Err(Error::InvalidInput(
                "cannot move directory into its own subtree".into(),
            ));
        }
        let now = Timestamp::now();
        let r = sqlx::query(
            "UPDATE entries SET parent_id = ?, name = ?, modified_at = ? WHERE id = ? AND kind = 0",
        )
        .bind(new_parent.raw())
        .bind(new_name)
        .bind(now)
        .bind(dir_id.raw())
        .execute(&mut *tx)
        .await;
        match r {
            Ok(r) if r.rows_affected() > 0 => {}
            Ok(_) => {
                return Err(Error::NotFound(format!("directory {}", dir_id.raw())));
            }
            Err(e) => return Err(Self::map_sqlite_constraint(e)),
        }
        tx.commit().await?;
        Ok(())
    }

    #[instrument(skip(self), fields(dir_path = %dir_path), err(Debug))]
    async fn ensure_directory_path(&self, dir_path: &str) -> Result<EntryId> {
        let segs = canonical_path_segments(dir_path)?;
        let mut cur = EntryId::from_raw(ROOT_ID);
        for seg in segs {
            let next: Option<i64> = sqlx::query_scalar(
                "SELECT id FROM entries WHERE parent_id = ? AND name = ? AND kind = 0",
            )
            .bind(cur.raw())
            .bind(&seg)
            .fetch_optional(&self.pool)
            .await?;
            cur = if let Some(id) = next {
                EntryId::from_raw(id)
            } else {
                let now = Timestamp::now();
                let id: i64 = sqlx::query_scalar(
                    r"INSERT INTO entries (parent_id, kind, name, description, content, size, created_at, modified_at, accessed_at)
                    VALUES (?, 0, ?, NULL, NULL, NULL, ?, ?, ?) RETURNING id",
                )
                .bind(cur.raw())
                .bind(&seg)
                .bind(now)
                .bind(now)
                .bind(now)
                .fetch_one(&self.pool)
                .await
                .map_err(Self::map_sqlite_constraint)?;
                EntryId::from_raw(id)
            };
        }
        Ok(cur)
    }

    #[instrument(skip(self), fields(path = %path), err(Debug))]
    async fn entry_description(&self, path: &str) -> Result<Option<String>> {
        let id = self.resolve_path(path, None).await?;
        let d: Option<String> = sqlx::query_scalar("SELECT description FROM entries WHERE id = ?")
            .bind(id.raw())
            .fetch_one(&self.pool)
            .await?;
        Ok(d)
    }

    #[instrument(skip(self, description), fields(path = %path), err(Debug))]
    async fn set_entry_description(&self, path: &str, description: Option<&str>) -> Result<()> {
        let id = self.resolve_path(path, None).await?;
        let now = Timestamp::now();
        let r = sqlx::query("UPDATE entries SET description = ?, modified_at = ? WHERE id = ?")
            .bind(description)
            .bind(now)
            .bind(id.raw())
            .execute(&self.pool)
            .await?;
        if r.rows_affected() == 0 {
            return Err(Error::NotFound(path.to_string()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cte_path_segments_json_roundtrip() {
        let s = vec!["a".into(), "b".into(), "42".into()];
        let j = path_segments_json(&s).unwrap();
        assert_eq!(j, r#"["a","b","42"]"#);
    }

    #[tokio::test]
    async fn cte_resolve_path_smoke() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("cte.db");
        let uri = format!("sqlite://{}", db_path.display());
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(
                uri.parse::<SqliteConnectOptions>()
                    .unwrap()
                    .create_if_missing(true)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        migrate(&pool).await.unwrap();
        let now = Timestamp::now();
        let id_a: i64 = sqlx::query_scalar(
            r"INSERT INTO entries (parent_id, kind, name, description, content, size, created_at, modified_at, accessed_at)
            VALUES (1, 0, 'alpha', NULL, NULL, NULL, ?, ?, ?) RETURNING id",
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            r"INSERT INTO entries (parent_id, kind, name, description, content, size, created_at, modified_at, accessed_at)
            VALUES (?, 0, 'beta', NULL, NULL, NULL, ?, ?, ?)",
        )
        .bind(id_a)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();
        let id = resolve_path_id(
            &pool,
            &["alpha".into(), "beta".into()],
            Some(EntryKind::Dir),
        )
        .await
        .unwrap();
        let path = canonical_path_for_id(&pool, id).await.unwrap();
        assert_eq!(path, "/alpha/beta");
        let root = resolve_path_id(&pool, &[], Some(EntryKind::Dir))
            .await
            .unwrap();
        assert_eq!(root.raw(), ROOT_ID);
    }
}

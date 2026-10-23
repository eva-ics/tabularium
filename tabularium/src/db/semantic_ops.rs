//! Semantic coordination, source validation, bounded background refresh and RRF fusion.

use super::semantic::{SemanticConfig, SemanticDocument, SemanticIndex};
use super::{Database, EntryId, SearchHit, Storage};
use crate::{AuthContext, Error, Result, SearchKind};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// Search results plus explicit degraded/partial-engine status.
pub struct SearchReport {
    hits: Vec<SearchHit>,
    lexical_status: &'static str,
    semantic_status: &'static str,
}
impl SearchReport {
    pub fn hits(&self) -> &[SearchHit] {
        &self.hits
    }
    pub fn into_hits(self) -> Vec<SearchHit> {
        self.hits
    }
    pub fn lexical_status(&self) -> &str {
        self.lexical_status
    }
    pub fn semantic_status(&self) -> &str {
        self.semantic_status
    }
}

impl<S: Storage> Database<S> {
    /// Attach an optional embeddings sidecar; no model calls or source migrations at startup.
    pub async fn with_semantic(
        mut self,
        config: SemanticConfig,
        default_dir: &Path,
    ) -> Result<Self> {
        self.semantic = Some(Arc::new(SemanticIndex::open(config, default_dir).await?));
        Ok(self)
    }
    pub(super) async fn queue_semantic(&self, id: EntryId) {
        if let Some(index) = &self.semantic
            && let Err(e) = index.queue(id.raw()).await
        {
            tracing::warn!(error=%e,"semantic refresh could not be queued; startup reconciliation or reindex required");
        }
    }
    async fn semantic_document(&self, id: EntryId) -> Result<SemanticDocument> {
        let before = self.storage.get_file_meta(id).await?;
        let body = self.storage.get_file_content(id).await?;
        let after = self.storage.get_file_meta(id).await?;
        if before.revision() != after.revision()
            || before.canonical_path() != after.canonical_path()
        {
            return Err(Error::Search(
                "document changed while preparing semantic index; retry".into(),
            ));
        }
        Ok(SemanticDocument {
            id: id.raw(),
            path: after.canonical_path().into(),
            revision: after
                .revision()
                .ok_or_else(|| Error::Search("document has no revision".into()))?
                .into(),
            body,
        })
    }
    /// Persistent dirty IDs survive restart; source revision/path checks prevent stale quotations.
    pub fn start_semantic_worker(self: &Arc<Self>)
    where
        S: 'static,
    {
        let Some(index) = self.semantic.clone() else {
            return;
        };
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            // Recover writes that committed before a process died while queuing sidecar work.
            if let Some(db) = weak.upgrade()
                && matches!(index.state().await, Ok("ready" | "partial"))
                && let Ok(rows) = db.storage.files_for_search_reindex(None).await
            {
                let mut ids: HashSet<i64> = rows.into_iter().map(|r| r.0.raw()).collect();
                if let Ok(stored) = index.indexed_ids().await {
                    ids.extend(stored);
                }
                for id in ids {
                    if let Err(e) = index.queue(id).await {
                        tracing::warn!(error=%e,"semantic startup reconciliation failed");
                        break;
                    }
                }
            }
            loop {
                let Some(db) = weak.upgrade() else { break };
                let count = match db.flush_semantic_pending().await {
                    Ok(n) => n,
                    Err(e) => {
                        tracing::warn!(error=%e,"semantic refresh pending; will retry");
                        0
                    }
                };
                drop(db);
                if count > 0 {
                    tokio::task::yield_now().await;
                    continue;
                }
                tokio::select! {_ = index.wake.notified()=>{},_ = tokio::time::sleep(Duration::from_secs(5))=>{}}
            }
        });
    }
    /// Process one bounded batch. Primarily useful for maintenance and deterministic tests.
    pub async fn flush_semantic_pending(&self) -> Result<usize> {
        let Some(index) = &self.semantic else {
            return Ok(0);
        };
        let _guard = index.operation.lock().await;
        if matches!(index.state().await?, "unindexed" | "incompatible") {
            return Ok(0);
        }
        let pending = index.pending(16).await?;
        let mut changed = Vec::new();
        let mut done = Vec::new();
        for &(id, generation) in &pending {
            match self.semantic_document(EntryId::from(id)).await {
                Ok(doc) => {
                    if !index.matches(&doc).await? {
                        changed.push(doc);
                    }
                    done.push((id, generation));
                }
                Err(Error::NotFound(_)) => {
                    index.remove(id).await?;
                    done.push((id, generation));
                }
                Err(e) => return Err(e),
            }
        }
        if !changed.is_empty() {
            index.replace(&changed, None, false).await?;
        }
        for (id, generation) in done {
            index.acknowledge(id, generation).await?;
        }
        Ok(pending.len())
    }
    /// Rebuild selected derived engines. Explicit semantic mode requires configuration.
    pub async fn reindex_kind(&self, path: Option<&str>, kind: SearchKind) -> Result<()> {
        if kind == SearchKind::Semantic && self.semantic.is_none() {
            return Err(Error::Search("semantic search is not configured".into()));
        }
        if kind != SearchKind::Semantic {
            self.reindex(path).await?;
        }
        if kind != SearchKind::Lexical
            && let Some(index) = &self.semantic
        {
            let _guard = index.operation.lock().await;
            if path.is_some_and(|p| !p.is_empty() && p != "/")
                && matches!(index.state().await?, "unindexed" | "incompatible")
            {
                return Err(Error::Search(
                    "first semantic reindex, or a model change, requires full reindex /".into(),
                ));
            }
            let rows = self.storage.files_for_search_reindex(path).await?;
            let mut documents = Vec::with_capacity(rows.len());
            for (id, _, _, _, _) in rows {
                documents.push(self.semantic_document(id).await?);
            }
            index.replace(&documents, path, true).await?;
            index.wake.notify_one();
        }
        Ok(())
    }
    async fn semantic_hits(
        &self,
        query: &str,
        path: Option<&str>,
        limit: usize,
        only: Option<EntryId>,
        auth: Option<&AuthContext>,
    ) -> Result<Vec<SearchHit>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let index = self
            .semantic
            .as_ref()
            .ok_or_else(|| Error::Search("semantic search is not configured".into()))?;
        let candidates = index
            .candidates(query, path, only.map(EntryId::raw))
            .await?;
        let mut seen = HashSet::new();
        let mut hits = Vec::new();
        for c in candidates {
            if seen.contains(&c.id) {
                continue;
            }
            let doc = match self.semantic_document(EntryId::from(c.id)).await {
                Ok(d) => d,
                Err(Error::NotFound(_)) => continue,
                Err(e) => return Err(e),
            };
            if doc.revision != c.revision {
                continue;
            }
            if let Some(prefix) = path.filter(|p| *p != "/" && !p.is_empty())
                && !doc
                    .path
                    .starts_with(&format!("{}/", prefix.trim_end_matches('/')))
            {
                continue;
            }
            if auth.is_some_and(|a| a.check_read_abs(&doc.path).is_err()) {
                continue;
            }
            let Some(snippet) = doc.body.get(c.start..c.end) else {
                return Err(Error::Search(
                    "semantic source offsets invalid; reindex required".into(),
                ));
            };
            hits.push(
                SearchHit::new(
                    EntryId::from(c.id),
                    doc.path,
                    snippet.into(),
                    c.score,
                    Some(c.line_start),
                )
                .with_source(doc.revision, c.line_end),
            );
            seen.insert(c.id);
            if hits.len() >= limit {
                break;
            }
        }
        Ok(hits)
    }
    /// Run engines concurrently, fuse document ranks, and expose partial/fallback status.
    pub async fn search_report(
        &self,
        query: &str,
        path: Option<&str>,
        limit: usize,
        only: Option<EntryId>,
        kind: SearchKind,
        auth: Option<&AuthContext>,
    ) -> Result<SearchReport> {
        let lexical = async {
            if kind == SearchKind::Semantic {
                return Ok(Vec::new());
            }
            let candidate_limit = if auth.is_some() { usize::MAX } else { limit };
            let hits = self.search_hits(query, path, candidate_limit, only).await?;
            Ok(hits
                .into_iter()
                .filter(|h| auth.is_none_or(|a| a.check_read_abs(h.path()).is_ok()))
                .take(limit)
                .collect::<Vec<_>>())
        };
        let semantic = async {
            if kind == SearchKind::Lexical || self.semantic.is_none() {
                return Ok(Vec::new());
            }
            let timeout = self
                .semantic
                .as_ref()
                .expect("checked")
                .config
                .search_timeout;
            tokio::time::timeout(
                Duration::from_secs(timeout),
                self.semantic_hits(query, path, limit, only, auth),
            )
            .await
            .map_err(|_| Error::Search("semantic search timed out".into()))?
        };
        let (lexical, semantic) = tokio::join!(lexical, semantic);
        let mut lexical_status = if kind == SearchKind::Semantic {
            "not_requested"
        } else {
            "ready"
        };
        let mut semantic_status = if kind == SearchKind::Lexical {
            "not_requested"
        } else if let Some(i) = &self.semantic {
            i.state().await.unwrap_or("unavailable")
        } else {
            "disabled"
        };
        if kind == SearchKind::Semantic && self.semantic.is_none() {
            return Err(Error::Search("semantic search is not configured".into()));
        }
        let mut lex = match lexical {
            Ok(h) => h,
            Err(e) => {
                if kind == SearchKind::Lexical || self.semantic.is_none() || semantic.is_err() {
                    return Err(e);
                }
                lexical_status = "unavailable";
                Vec::new()
            }
        };
        let sem = match semantic {
            Ok(h) => h,
            Err(e) => {
                if kind == SearchKind::Semantic {
                    return Err(e);
                }
                if !matches!(semantic_status, "unindexed" | "incompatible") {
                    semantic_status = "unavailable";
                }
                tracing::warn!(error=%e,"hybrid search falling back to lexical");
                Vec::new()
            }
        };
        let hits = if kind == SearchKind::Semantic {
            sem
        } else if kind == SearchKind::Lexical || sem.is_empty() {
            lex
        } else {
            let mut scores: HashMap<i64, f32> = HashMap::new();
            for list in [&lex, &sem] {
                for (rank, h) in list.iter().enumerate() {
                    *scores.entry(h.document_id().raw()).or_default() +=
                        1.0 / (60.0 + rank as f32 + 1.0);
                }
            }
            // Prefer the exact semantic excerpt for a document present in both lists.
            let mut by_id: HashMap<i64, SearchHit> =
                lex.drain(..).map(|h| (h.document_id().raw(), h)).collect();
            for h in sem {
                by_id.insert(h.document_id().raw(), h);
            }
            let mut all: Vec<_> = by_id.into_values().collect();
            for h in &mut all {
                h.set_score(scores[&h.document_id().raw()]);
            }
            all.sort_by(|a, b| {
                b.score()
                    .total_cmp(&a.score())
                    .then_with(|| a.path().cmp(b.path()))
            });
            all.truncate(limit);
            all
        };
        Ok(SearchReport {
            hits,
            lexical_status,
            semantic_status,
        })
    }
}

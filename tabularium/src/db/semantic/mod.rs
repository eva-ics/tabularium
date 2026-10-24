//! Rebuildable SQLite semantic sidecar. The authoritative document DB is never migrated.

mod chunk;
mod client;
pub use client::SemanticConfig;

use crate::{Error, Result};
use client::EmbeddingsClient;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};

pub(super) struct SemanticIndex {
    pub config: SemanticConfig,
    pool: SqlitePool,
    client: EmbeddingsClient,
    pub operation: Mutex<()>,
    pub wake: Arc<Notify>,
}

pub(super) struct SemanticDocument {
    pub id: i64,
    pub path: String,
    pub revision: String,
    pub body: String,
}

pub(super) struct SemanticCandidate {
    pub id: i64,
    pub revision: String,
    pub start: usize,
    pub end: usize,
    pub line_start: usize,
    pub line_end: usize,
    pub score: f32,
}

impl SemanticIndex {
    pub async fn open(config: SemanticConfig, default_dir: &Path) -> Result<Self> {
        let client = EmbeddingsClient::new(&config)?;
        let dir = config.index_dir.as_deref().unwrap_or(default_dir);
        tokio::fs::create_dir_all(dir).await?;
        let options = SqliteConnectOptions::new()
            .filename(dir.join("semantic.db"))
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await?;
        // Schema belongs only to disposable derived storage, not tabularium.db.
        for statement in [
            "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            "CREATE TABLE IF NOT EXISTS docs (id INTEGER PRIMARY KEY, path TEXT NOT NULL, revision TEXT NOT NULL)",
            "CREATE TABLE IF NOT EXISTS chunks (doc_id INTEGER NOT NULL REFERENCES docs(id) ON DELETE CASCADE, start INTEGER NOT NULL, end INTEGER NOT NULL, line_start INTEGER NOT NULL, line_end INTEGER NOT NULL, vector BLOB NOT NULL, PRIMARY KEY(doc_id,start))",
            "CREATE TABLE IF NOT EXISTS dirty (id INTEGER PRIMARY KEY, generation INTEGER NOT NULL DEFAULT 1)",
            "CREATE TABLE IF NOT EXISTS staging_chunks (doc_id INTEGER NOT NULL, start INTEGER NOT NULL, end INTEGER NOT NULL, line_start INTEGER NOT NULL, line_end INTEGER NOT NULL, vector BLOB NOT NULL, PRIMARY KEY(doc_id,start))",
        ] {
            sqlx::query(statement).execute(&pool).await?;
        }
        Ok(Self {
            config,
            pool,
            client,
            operation: Mutex::new(()),
            wake: Arc::new(Notify::new()),
        })
    }
    async fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar("SELECT value FROM meta WHERE key=?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?)
    }
    pub async fn state(&self) -> Result<&'static str> {
        match self.meta("fingerprint").await? {
            None => Ok("unindexed"),
            Some(f) if f != self.config.fingerprint() => Ok("incompatible"),
            Some(_) => {
                let dirty: i64 = sqlx::query_scalar("SELECT count(*) FROM dirty")
                    .fetch_one(&self.pool)
                    .await?;
                if dirty > 0 || self.meta("complete").await?.as_deref() != Some("true") {
                    Ok("partial")
                } else {
                    Ok("ready")
                }
            }
        }
    }
    pub async fn queue(&self, id: i64) -> Result<()> {
        sqlx::query("INSERT INTO dirty(id,generation) VALUES(?,1) ON CONFLICT(id) DO UPDATE SET generation=generation+1").bind(id).execute(&self.pool).await?;
        self.wake.notify_one();
        Ok(())
    }
    pub async fn pending(&self, limit: i64) -> Result<Vec<(i64, i64)>> {
        Ok(
            sqlx::query_as("SELECT id,generation FROM dirty ORDER BY id LIMIT ?")
                .bind(limit)
                .fetch_all(&self.pool)
                .await?,
        )
    }
    pub async fn acknowledge(&self, id: i64, generation: i64) -> Result<()> {
        sqlx::query("DELETE FROM dirty WHERE id=? AND generation=?")
            .bind(id)
            .bind(generation)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn indexed_ids(&self) -> Result<Vec<i64>> {
        Ok(sqlx::query_scalar("SELECT id FROM docs")
            .fetch_all(&self.pool)
            .await?)
    }
    pub async fn matches(&self, doc: &SemanticDocument) -> Result<bool> {
        let value: Option<(String, String)> =
            sqlx::query_as("SELECT path,revision FROM docs WHERE id=?")
                .bind(doc.id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(value.is_some_and(|(p, r)| p == doc.path && r == doc.revision))
    }
    pub async fn remove(&self, id: i64) -> Result<()> {
        sqlx::query("DELETE FROM docs WHERE id=?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    /// No authoritative state is changed until all embeddings have succeeded.
    pub async fn replace(
        &self,
        documents: &[SemanticDocument],
        subtree: Option<&str>,
        rebuild: bool,
    ) -> Result<()> {
        // The operation mutex serializes rebuilds and background updates. Stage one
        // bounded HTTP batch at a time; never hold a DB transaction across HTTP.
        sqlx::query("DELETE FROM staging_chunks")
            .execute(&self.pool)
            .await?;
        let mut chunks = Vec::with_capacity(self.config.batch_size);
        let mut inputs = Vec::with_capacity(self.config.batch_size);
        let mut dim = None;
        let stored_dim = if rebuild && subtree.is_none_or(|p| p.is_empty() || p == "/") {
            None
        } else {
            self.meta("dimension").await?
        };
        let was_complete = self.meta("complete").await?;
        for doc in documents {
            for c in chunk::split(
                &doc.body,
                self.config.chunk_bytes,
                self.config.overlap_bytes,
            ) {
                inputs.push(format!(
                    "{}{}",
                    self.config.document_prefix,
                    &doc.body[c.start..c.end]
                ));
                chunks.push((doc.id, c));
                if inputs.len() == self.config.batch_size {
                    self.stage_batch(&inputs, &chunks, &mut dim, stored_dim.as_deref())
                        .await?;
                    inputs.clear();
                    chunks.clear();
                }
            }
        }
        if !inputs.is_empty() {
            self.stage_batch(&inputs, &chunks, &mut dim, stored_dim.as_deref())
                .await?;
        }
        let mut tx = self.pool.begin().await?;
        if rebuild {
            if let Some(prefix) = subtree.filter(|p| !p.is_empty() && *p != "/") {
                let prefix = prefix.trim_end_matches('/');
                sqlx::query("DELETE FROM docs WHERE path=? OR substr(path,1,length(?)+1)=?||'/'")
                    .bind(prefix)
                    .bind(prefix)
                    .bind(prefix)
                    .execute(&mut *tx)
                    .await?;
            } else {
                sqlx::query("DELETE FROM docs").execute(&mut *tx).await?;
            }
        }
        for doc in documents {
            sqlx::query("DELETE FROM docs WHERE id=?")
                .bind(doc.id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO docs(id,path,revision) VALUES(?,?,?)")
                .bind(doc.id)
                .bind(&doc.path)
                .bind(&doc.revision)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("INSERT INTO chunks SELECT doc_id,start,end,line_start,line_end,vector FROM staging_chunks")
            .execute(&mut *tx).await?;
        sqlx::query("DELETE FROM staging_chunks")
            .execute(&mut *tx)
            .await?;
        if rebuild && dim.is_none() && subtree.is_none_or(|p| p.is_empty() || p == "/") {
            sqlx::query("DELETE FROM meta WHERE key='dimension'")
                .execute(&mut *tx)
                .await?;
        }
        if let Some(dim) = dim {
            sqlx::query("INSERT OR REPLACE INTO meta(key,value) VALUES('dimension',?)")
                .bind(dim.to_string())
                .execute(&mut *tx)
                .await?;
        }
        if rebuild {
            sqlx::query("INSERT OR REPLACE INTO meta(key,value) VALUES('fingerprint',?)")
                .bind(self.config.fingerprint())
                .execute(&mut *tx)
                .await?;
            let complete = subtree.is_none_or(|p| p.is_empty() || p == "/");
            if complete || was_complete.is_none() {
                sqlx::query("INSERT OR REPLACE INTO meta(key,value) VALUES('complete',?)")
                    .bind(if complete { "true" } else { "false" })
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }
    async fn stage_batch(
        &self,
        inputs: &[String],
        chunks: &[(i64, chunk::TextChunk)],
        dimension: &mut Option<usize>,
        stored_dimension: Option<&str>,
    ) -> Result<()> {
        let vectors = self.client.embed(inputs).await?;
        for vector in &vectors {
            let dim = vector.len();
            if dimension.is_some_and(|d| d != dim)
                || stored_dimension.is_some_and(|d| d != dim.to_string())
            {
                return Err(Error::Search(
                    "embedding dimensions changed; run full semantic reindex".into(),
                ));
            }
            *dimension = Some(dim);
        }
        let mut tx = self.pool.begin().await?;
        for ((id, c), vector) in chunks.iter().zip(vectors) {
            let bytes: Vec<u8> = vector.into_iter().flat_map(f32::to_le_bytes).collect();
            sqlx::query("INSERT INTO staging_chunks(doc_id,start,end,line_start,line_end,vector) VALUES(?,?,?,?,?,?)")
                .bind(id).bind(c.start as i64).bind(c.end as i64)
                .bind(c.line_start as i64).bind(c.line_end as i64).bind(bytes)
                .execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn candidates(
        &self,
        query: &str,
        subtree: Option<&str>,
        only: Option<i64>,
    ) -> Result<Vec<SemanticCandidate>> {
        if matches!(self.state().await?, "unindexed" | "incompatible") {
            return Err(Error::Search(
                "semantic index missing or incompatible; run reindex --kind semantic /".into(),
            ));
        }
        let vectors = self
            .client
            .embed(&[format!("{}{}", self.config.query_prefix, query)])
            .await?;
        let q = &vectors[0];
        if self
            .meta("dimension")
            .await?
            .is_some_and(|d| d != q.len().to_string())
        {
            return Err(Error::Search(
                "embedding dimensions changed; run full semantic reindex".into(),
            ));
        }
        let prefix = subtree.unwrap_or("/").trim_end_matches('/');
        let rows=sqlx::query("SELECT c.doc_id,d.revision,c.start,c.end,c.line_start,c.line_end,c.vector FROM chunks c JOIN docs d ON d.id=c.doc_id WHERE (? IS NULL OR d.id=?) AND (?='' OR substr(d.path,1,length(?)+1)=?||'/')").bind(only).bind(only).bind(prefix).bind(prefix).bind(prefix).fetch_all(&self.pool).await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let bytes: Vec<u8> = row.try_get("vector")?;
            if bytes.len() != q.len() * 4 {
                return Err(Error::Search(
                    "invalid semantic vector dimensions; rebuild index".into(),
                ));
            }
            let mut score = 0.0;
            for (part, &v) in bytes.chunks_exact(4).zip(q) {
                score += f32::from_le_bytes(part.try_into().expect("four-byte chunk")) * v;
            }
            if !score.is_finite() {
                return Err(Error::Search(
                    "invalid semantic vector values; rebuild index".into(),
                ));
            }
            let start: i64 = row.try_get("start")?;
            let end: i64 = row.try_get("end")?;
            let ls: i64 = row.try_get("line_start")?;
            let le: i64 = row.try_get("line_end")?;
            if start < 0 || end < start || ls < 1 || le < ls {
                return Err(Error::Search(
                    "invalid semantic offsets; rebuild index".into(),
                ));
            }
            out.push(SemanticCandidate {
                id: row.try_get("doc_id")?,
                revision: row.try_get("revision")?,
                start: start as usize,
                end: end as usize,
                line_start: ls as usize,
                line_end: le as usize,
                score,
            });
        }
        out.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.id.cmp(&b.id))
                .then_with(|| a.start.cmp(&b.start))
        });
        Ok(out)
    }
}

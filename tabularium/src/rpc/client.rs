//! Typed JSON-RPC 2.0 client over HTTP POST.

use std::path::Path;
use std::time::Duration;

use reqwest::header::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::resource_path::normalize_path_for_rpc;
use crate::text_lines::TailMode;
use crate::{EntryId, Error, Result, Timestamp};

use crate::jsonrpc_codes::DUPLICATE_RESOURCE;

/// HTTP JSON-RPC client for a tabularium server (`POST` to `{base}/rpc`).
pub struct Client {
    base: String,
    http: reqwest::Client,
    extra_headers: HeaderMap,
}

impl Client {
    /// `api_uri` is e.g. `http://127.0.0.1:3050` (no trailing slash).
    pub fn init(api_uri: impl Into<String>, timeout: Duration) -> Result<Self> {
        Self::init_with_extra_headers(api_uri, timeout, HeaderMap::new())
    }

    /// Same as [`Self::init`], with extra headers on every RPC `POST` (and the same map should be passed to [`crate::ws::Client::connect_with_headers`]).
    pub fn init_with_extra_headers(
        api_uri: impl Into<String>,
        timeout: Duration,
        extra_headers: HeaderMap,
    ) -> Result<Self> {
        let base = api_uri.into().trim_end_matches('/').to_string();
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .default_headers(extra_headers.clone())
            .build()
            .map_err(|e| Error::InvalidInput(e.to_string()))?;
        Ok(Self {
            base,
            http,
            extra_headers,
        })
    }

    /// Base URL passed to [`Self::init`] (no trailing slash), e.g. `http://127.0.0.1:3050`.
    pub fn api_base(&self) -> &str {
        &self.base
    }

    /// Extra headers applied to RPC requests (mirror into WebSocket connect).
    pub fn extra_headers(&self) -> &HeaderMap {
        &self.extra_headers
    }

    /// New client for the same API base with a different HTTP timeout.
    pub fn with_timeout(&self, timeout: Duration) -> Result<Self> {
        Self::init_with_extra_headers(self.api_base(), timeout, self.extra_headers.clone())
    }

    fn url(&self) -> String {
        format!("{}/rpc", self.base)
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let body = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1_i64,
        });
        let res = self
            .http
            .post(self.url())
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Io(e.to_string()))?;
        if !res.status().is_success() {
            return Err(Error::InvalidInput(format!("rpc http {}", res.status())));
        }
        let v: Value = res.json().await.map_err(|e| Error::Io(e.to_string()))?;
        if let Some(err) = v.get("error") {
            let code_raw = err
                .get("code")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(-32603);
            let code = i32::try_from(code_raw).unwrap_or(-32603);
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("rpc error")
                .to_string();
            return Err(if code == DUPLICATE_RESOURCE {
                let detail = msg
                    .strip_prefix("duplicate: ")
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(msg.as_str());
                Error::Duplicate(detail.to_string())
            } else {
                Error::InvalidInput(msg)
            });
        }
        v.get("result")
            .cloned()
            .ok_or_else(|| Error::InvalidInput("rpc missing result".into()))
    }

    /// `test` RPC — server product identity and process uptime (nanoseconds on the wire, `u64`).
    pub async fn test(&self) -> Result<ServerTest> {
        let r = self.call("test", json!({})).await?;
        serde_json::from_value(r).map_err(|e| Error::InvalidInput(e.to_string()))
    }

    /// `list_directory` RPC (`path` is absolute, e.g. `/` for root).
    pub async fn list_directory(&self, path: impl AsRef<Path>) -> Result<Vec<ListedEntryRow>> {
        let path = normalize_path_for_rpc(path)?;
        let params = json!({ "path": path });
        let r = self.call("list_directory", params).await?;
        serde_json::from_value(r).map_err(|e| Error::InvalidInput(e.to_string()))
    }

    /// Directories at repository root (`list_directory("/")` filtered to `kind == 0`).
    pub async fn list_root_directories(&self) -> Result<Vec<ListedEntryRow>> {
        Ok(self
            .list_directory("/")
            .await?
            .into_iter()
            .filter(ListedEntryRow::is_directory)
            .collect())
    }

    /// `search` RPC; `subtree` is an absolute directory prefix filter, or `None` for all documents.
    pub async fn search(
        &self,
        query: impl AsRef<str>,
        subtree: Option<&Path>,
    ) -> Result<Vec<SearchHitRow>> {
        let params = match subtree {
            None => json!({ "query": query.as_ref() }),
            Some(p) => {
                let ps = normalize_path_for_rpc(p)?;
                json!({ "query": query.as_ref(), "path": ps })
            }
        };
        let r = self.call("search", params).await?;
        serde_json::from_value(r).map_err(|e| Error::InvalidInput(e.to_string()))
    }

    /// `exists` RPC — whether `path` resolves to an existing **file**.
    pub async fn document_exists(&self, path: impl AsRef<Path>) -> Result<bool> {
        let path = normalize_path_for_rpc(path)?;
        let params = json!({ "path": path });
        let r = self.call("exists", params).await?;
        serde_json::from_value(r).map_err(|e| Error::InvalidInput(e.to_string()))
    }

    /// `get_document` RPC; `path` is an absolute file path (e.g. `/notes/readme`).
    pub async fn get_document(&self, path: impl AsRef<Path>) -> Result<DocumentBody> {
        let path = normalize_path_for_rpc(path)?;
        let params = json!({ "path": path });
        let r = self.call("get_document", params).await?;
        serde_json::from_value(r).map_err(|e| Error::InvalidInput(e.to_string()))
    }

    /// `create_directory` RPC; `path` is absolute (e.g. `/notes`).
    pub async fn create_directory(
        &self,
        path: impl AsRef<Path>,
        description: Option<&str>,
    ) -> Result<EntryId> {
        let path = normalize_path_for_rpc(path)?;
        let params = json!({ "path": path, "description": description });
        let r = self.call("create_directory", params).await?;
        serde_json::from_value(r).map_err(|e| Error::InvalidInput(e.to_string()))
    }

    /// `describe` RPC — read `description` metadata when `description` param is omitted.
    pub async fn describe_entry(&self, path: impl AsRef<Path>) -> Result<Option<String>> {
        let path = normalize_path_for_rpc(path)?;
        let v = self.call("describe", json!({ "path": path })).await?;
        let o: DescribeResult =
            serde_json::from_value(v).map_err(|e| Error::InvalidInput(e.to_string()))?;
        Ok(o.description)
    }

    /// `describe` RPC — set or clear description (`""` clears) when `description` param is sent.
    pub async fn set_entry_description(
        &self,
        path: impl AsRef<Path>,
        description: impl AsRef<str>,
    ) -> Result<()> {
        let path = normalize_path_for_rpc(path)?;
        self.call(
            "describe",
            json!({
                "path": path,
                "description": description.as_ref(),
            }),
        )
        .await?;
        Ok(())
    }

    /// `create_document` RPC; `path` is an absolute file path (parent directory must exist).
    pub async fn create_document(
        &self,
        path: impl AsRef<Path>,
        content: impl AsRef<str>,
    ) -> Result<EntryId> {
        let path = normalize_path_for_rpc(path)?;
        let params = json!({
            "path": path,
            "content": content.as_ref(),
        });
        let r = self.call("create_document", params).await?;
        serde_json::from_value(r).map_err(|e| Error::InvalidInput(e.to_string()))
    }

    /// `put_document` RPC — create document, or replace body if it already exists.
    pub async fn put_document(
        &self,
        path: impl AsRef<Path>,
        content: impl AsRef<str>,
    ) -> Result<()> {
        let path = normalize_path_for_rpc(path)?;
        let params = json!({
            "path": path,
            "content": content.as_ref(),
        });
        self.call("put_document", params).await?;
        Ok(())
    }

    /// `append_document` RPC.
    pub async fn append_document(
        &self,
        path: impl AsRef<Path>,
        content: impl AsRef<str>,
    ) -> Result<()> {
        let path = normalize_path_for_rpc(path)?;
        let params = json!({
            "path": path,
            "content": content.as_ref(),
        });
        self.call("append_document", params).await?;
        Ok(())
    }

    /// `say_document` RPC — server appends a markdown chat block (`## from_id`, body, trailing blank line).
    pub async fn say_document(
        &self,
        path: impl AsRef<Path>,
        from_id: impl AsRef<str>,
        content: impl AsRef<str>,
    ) -> Result<()> {
        let path = normalize_path_for_rpc(path)?;
        let params = json!({
            "path": path,
            "from_id": from_id.as_ref(),
            "content": content.as_ref(),
        });
        self.call("say_document", params).await?;
        Ok(())
    }

    /// `touch_document` RPC — create empty file or bump `modified_at`; with `Some(ts)` set exact `modified_at` (creates empty file if missing).
    pub async fn touch_document(
        &self,
        path: impl AsRef<Path>,
        modified_at: Option<Timestamp>,
    ) -> Result<()> {
        let path = normalize_path_for_rpc(path)?;
        let params = match modified_at {
            None => json!({ "path": path }),
            Some(ts) => json!({ "path": path, "modified_at": ts }),
        };
        self.call("touch_document", params).await?;
        Ok(())
    }

    /// `delete_document` RPC.
    pub async fn delete_document(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = normalize_path_for_rpc(path)?;
        let params = json!({ "path": path });
        self.call("delete_document", params).await?;
        Ok(())
    }

    /// `delete_directory` RPC; `path` is absolute.
    pub async fn delete_directory(&self, path: impl AsRef<Path>, recursive: bool) -> Result<()> {
        let path = normalize_path_for_rpc(path)?;
        let params = json!({
            "path": path,
            "recursive": recursive,
        });
        self.call("delete_directory", params).await?;
        Ok(())
    }

    /// Long-poll `wait` RPC until the document body changes or the server long-poll timeout.
    pub async fn wait_document(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = normalize_path_for_rpc(path)?;
        let params = json!({ "path": path });
        self.call("wait", params).await?;
        Ok(())
    }

    /// List only **files** under `path` (absolute directory path).
    pub async fn list_documents(&self, path: impl AsRef<Path>) -> Result<Vec<DocumentMetaRow>> {
        let dir = normalize_path_for_rpc(path)?;
        let base = if dir == "/" {
            String::new()
        } else {
            dir.trim_end_matches('/').to_string()
        };
        let rows = self.list_directory(Path::new(&dir)).await?;
        Ok(rows
            .into_iter()
            .filter(|r| r.kind == 1)
            .map(|r| {
                let full = if base.is_empty() {
                    format!("/{}", r.name)
                } else {
                    format!("{}/{}", base, r.name)
                };
                DocumentMetaRow {
                    id: r.id,
                    name: r.name,
                    path: full,
                    created_at: r.created_at,
                    modified_at: r.modified_at,
                    accessed_at: r.accessed_at,
                    size_bytes: r.size_bytes.unwrap_or(0),
                }
            })
            .collect())
    }

    /// `head` RPC — first `lines` logical lines.
    pub async fn document_head(&self, path: impl AsRef<Path>, lines: u32) -> Result<String> {
        let path = normalize_path_for_rpc(path)?;
        let params = json!({ "path": path, "lines": lines });
        let r = self.call("head", params).await?;
        let t: TextPayload =
            serde_json::from_value(r).map_err(|e| Error::InvalidInput(e.to_string()))?;
        Ok(t.text)
    }

    /// `tail` RPC — [`TailMode::Last`] / [`TailMode::FromLine`] (`"+N"` on the wire).
    pub async fn document_tail(&self, path: impl AsRef<Path>, mode: TailMode) -> Result<String> {
        let path = normalize_path_for_rpc(path)?;
        let lines_val = match mode {
            TailMode::Last(n) => json!(n),
            TailMode::FromLine(n) => json!(format!("+{n}")),
        };
        let params = json!({ "path": path, "lines": lines_val });
        let r = self.call("tail", params).await?;
        let t: TextPayload =
            serde_json::from_value(r).map_err(|e| Error::InvalidInput(e.to_string()))?;
        Ok(t.text)
    }

    /// `slice` RPC — inclusive 1-based line range.
    pub async fn document_slice(
        &self,
        path: impl AsRef<Path>,
        start_line: u32,
        end_line: u32,
    ) -> Result<String> {
        let path = normalize_path_for_rpc(path)?;
        let params = json!({
            "path": path,
            "start_line": start_line,
            "end_line": end_line,
        });
        let r = self.call("slice", params).await?;
        let t: TextPayload =
            serde_json::from_value(r).map_err(|e| Error::InvalidInput(e.to_string()))?;
        Ok(t.text)
    }

    /// `grep` RPC. `max_matches == 0` means unlimited (server contract).
    pub async fn document_grep(
        &self,
        path: impl AsRef<Path>,
        pattern: impl AsRef<str>,
        max_matches: u64,
        invert_match: bool,
    ) -> Result<Vec<GrepLineRow>> {
        let path = normalize_path_for_rpc(path)?;
        let params = json!({
            "path": path,
            "pattern": pattern.as_ref(),
            "max_matches": max_matches,
            "invert_match": invert_match,
        });
        let r = self.call("grep", params).await?;
        serde_json::from_value(r).map_err(|e| Error::InvalidInput(e.to_string()))
    }

    /// `stat` RPC.
    pub async fn document_stat(&self, path: impl AsRef<Path>) -> Result<StatRow> {
        let path = normalize_path_for_rpc(path)?;
        let params = json!({ "path": path });
        let r = self.call("stat", params).await?;
        serde_json::from_value(r).map_err(|e| Error::InvalidInput(e.to_string()))
    }

    /// `wc` RPC.
    pub async fn document_wc(&self, path: impl AsRef<Path>) -> Result<WcRow> {
        let path = normalize_path_for_rpc(path)?;
        let params = json!({ "path": path });
        let r = self.call("wc", params).await?;
        serde_json::from_value(r).map_err(|e| Error::InvalidInput(e.to_string()))
    }

    /// `rename_directory` RPC; both paths are absolute; same parent (rename last segment).
    pub async fn rename_directory(
        &self,
        path: impl AsRef<Path>,
        new_path: impl AsRef<Path>,
    ) -> Result<()> {
        let path = normalize_path_for_rpc(path)?;
        let new_path = normalize_path_for_rpc(new_path)?;
        let params = json!({
            "path": path,
            "new_path": new_path,
        });
        self.call("rename_directory", params).await?;
        Ok(())
    }

    /// `rename_document` RPC; `path` is an absolute file path.
    pub async fn rename_document(
        &self,
        path: impl AsRef<Path>,
        new_name: impl AsRef<str>,
    ) -> Result<()> {
        let path = normalize_path_for_rpc(path)?;
        let params = json!({
            "path": path,
            "new_name": new_name.as_ref(),
        });
        self.call("rename_document", params).await?;
        Ok(())
    }

    /// `move_document` RPC; `new_path` is the destination **file** path (absolute).
    pub async fn move_document(
        &self,
        path: impl AsRef<Path>,
        new_path: impl AsRef<Path>,
    ) -> Result<()> {
        let path = normalize_path_for_rpc(path)?;
        let new_path = normalize_path_for_rpc(new_path)?;
        let params = json!({
            "path": path,
            "new_path": new_path,
        });
        self.call("move_document", params).await?;
        Ok(())
    }

    /// `move_directory` RPC; relocate a directory under `new_parent` with `new_name`.
    pub async fn move_directory(
        &self,
        path: impl AsRef<Path>,
        new_parent: impl AsRef<Path>,
        new_name: impl AsRef<str>,
    ) -> Result<()> {
        let path = normalize_path_for_rpc(path)?;
        let new_parent = normalize_path_for_rpc(new_parent)?;
        let params = json!({
            "path": path,
            "new_parent": new_parent,
            "new_name": new_name.as_ref(),
        });
        self.call("move_directory", params).await?;
        Ok(())
    }

    /// `update_document` RPC — replace full body (`PUT` semantics).
    pub async fn replace_document(
        &self,
        path: impl AsRef<Path>,
        content: impl AsRef<str>,
    ) -> Result<()> {
        let path = normalize_path_for_rpc(path)?;
        let params = json!({
            "path": path,
            "content": content.as_ref(),
        });
        self.call("update_document", params).await?;
        Ok(())
    }

    /// `reindex` RPC. `None` rebuilds the whole index from SQLite.
    pub async fn reindex(&self, path: Option<&Path>) -> Result<()> {
        let params = match path {
            None => json!({}),
            Some(p) => {
                let ps = normalize_path_for_rpc(p)?;
                json!({ "path": ps })
            }
        };
        self.call("reindex", params).await?;
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TextPayload {
    text: String,
}

#[derive(Debug, Clone, Deserialize)]
struct DescribeResult {
    description: Option<String>,
}

/// One line from `grep` RPC.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct GrepLineRow {
    line: usize,
    text: String,
}

impl GrepLineRow {
    pub fn line(&self) -> usize {
        self.line
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

/// `test` RPC payload (server diagnostics). `uptime` is nanoseconds (`u64`).
#[derive(Debug, Clone, Deserialize)]
pub struct ServerTest {
    product_name: String,
    product_version: String,
    uptime: u64,
}

impl ServerTest {
    pub fn product_name(&self) -> &str {
        &self.product_name
    }

    pub fn product_version(&self) -> &str {
        &self.product_version
    }

    /// Process uptime in nanoseconds since server `run()` began (`u64`; saturates at `u64::MAX`).
    pub fn uptime(&self) -> u64 {
        self.uptime
    }
}

/// `stat` RPC payload.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct StatRow {
    id: EntryId,
    path: String,
    size_bytes: i64,
    line_count: usize,
    created_at: Timestamp,
    modified_at: Timestamp,
    accessed_at: Timestamp,
}

impl StatRow {
    pub fn id(&self) -> EntryId {
        self.id
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// Parent directory path (absolute).
    pub fn directory_path(&self) -> &str {
        self.path
            .rsplit_once('/')
            .map_or("/", |(d, _)| if d.is_empty() { "/" } else { d })
    }

    /// File name segment (last path component).
    pub fn name(&self) -> &str {
        self.path
            .rsplit_once('/')
            .map_or(self.path.as_str(), |(_, n)| n)
    }

    pub fn size_bytes(&self) -> i64 {
        self.size_bytes
    }

    pub fn line_count(&self) -> usize {
        self.line_count
    }

    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub fn modified_at(&self) -> Timestamp {
        self.modified_at
    }

    pub fn accessed_at(&self) -> Timestamp {
        self.accessed_at
    }
}

/// `wc` RPC payload.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct WcRow {
    bytes: u64,
    lines: usize,
    words: usize,
    chars: usize,
}

impl WcRow {
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn lines(&self) -> usize {
        self.lines
    }

    pub fn words(&self) -> usize {
        self.words
    }

    pub fn chars(&self) -> usize {
        self.chars
    }
}

/// One row from `list_directory`.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ListedEntryRow {
    pub id: EntryId,
    pub kind: i64,
    pub name: String,
    pub description: Option<String>,
    pub created_at: Timestamp,
    pub modified_at: Timestamp,
    pub accessed_at: Timestamp,
    #[serde(default)]
    pub size_bytes: Option<i64>,
    #[serde(default)]
    pub recursive_file_count: u64,
}

impl ListedEntryRow {
    pub fn id(&self) -> EntryId {
        self.id
    }

    pub fn kind(&self) -> i64 {
        self.kind
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn is_directory(&self) -> bool {
        self.kind == 0
    }

    pub fn is_file(&self) -> bool {
        self.kind == 1
    }

    pub fn recursive_file_count(&self) -> u64 {
        self.recursive_file_count
    }
}

/// Document listing row from `list_documents`.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct DocumentMetaRow {
    id: EntryId,
    name: String,
    path: String,
    created_at: Timestamp,
    modified_at: Timestamp,
    accessed_at: Timestamp,
    size_bytes: i64,
}

impl DocumentMetaRow {
    pub fn id(&self) -> EntryId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub fn modified_at(&self) -> Timestamp {
        self.modified_at
    }

    pub fn accessed_at(&self) -> Timestamp {
        self.accessed_at
    }

    pub fn size_bytes(&self) -> i64 {
        self.size_bytes
    }
}

/// Search hit from `search`.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SearchHitRow {
    document_id: EntryId,
    path: String,
    snippet: String,
    score: f32,
    #[serde(default)]
    line_number: Option<usize>,
}

impl SearchHitRow {
    pub fn document_id(&self) -> EntryId {
        self.document_id
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// Parent directory path (absolute) of the hit file.
    pub fn parent_directory_path(&self) -> &str {
        self.path
            .rsplit_once('/')
            .map_or("/", |(d, _)| if d.is_empty() { "/" } else { d })
    }

    pub fn document(&self) -> &str {
        self.path
            .rsplit_once('/')
            .map_or(self.path.as_str(), |(_, n)| n)
    }

    pub fn snippet(&self) -> &str {
        &self.snippet
    }

    pub fn score(&self) -> f32 {
        self.score
    }

    pub fn line_number(&self) -> Option<usize> {
        self.line_number
    }
}

/// Full document payload from `get_document`.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct DocumentBody {
    id: EntryId,
    path: String,
    content: String,
    created_at: Timestamp,
    modified_at: Timestamp,
    accessed_at: Timestamp,
    size_bytes: i64,
}

impl DocumentBody {
    pub fn id(&self) -> EntryId {
        self.id
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn name(&self) -> &str {
        self.path
            .rsplit_once('/')
            .map_or(self.path.as_str(), |(_, n)| n)
    }

    /// Parent directory path (absolute) of the document.
    pub fn parent_directory_path(&self) -> &str {
        self.path
            .rsplit_once('/')
            .map_or("/", |(d, _)| if d.is_empty() { "/" } else { d })
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub fn modified_at(&self) -> Timestamp {
        self.modified_at
    }

    pub fn accessed_at(&self) -> Timestamp {
        self.accessed_at
    }

    pub fn size_bytes(&self) -> i64 {
        self.size_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_init_accepts_https_base() {
        Client::init("https://127.0.0.1:9", Duration::from_secs(1)).unwrap();
    }
}

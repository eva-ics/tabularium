//! MCP streamable HTTP (`/mcp`) — transport glue to the same JSON-RPC semantics as [`crate::web::dispatch_app_rpc`].

use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use http::request::Parts;
use rmcp::{
    ServerHandler,
    handler::server::tool::Extension,
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::{
        StreamableHttpServerConfig,
        streamable_http_server::{
            session::local::LocalSessionManager, tower::StreamableHttpService,
        },
    },
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, field, info, info_span};

use crate::auth::{RequestAuth, resolve_request_auth_arc};
use crate::otel::{enter_active_tool, record_tool_call};
use crate::web::{AppState, RpcAppError, dispatch_app_rpc};

#[derive(Debug, Deserialize, JsonSchema)]
struct MetadataArg {
    path: String,
    metadata: Value,
    #[serde(default)]
    only_if_revision: Option<String>,
}

const MCP_HELP_BASE: &str = include_str!("../res/mcp/help.txt");

const TOOL_NAMES_LINE: &str = "Call tool `methods` for the live catalog; the registered set depends on config `mcp.full` (destructive tools, ACL/PSK admin tools, and `whoami` are omitted unless full mode is enabled).";

/// JSON-RPC / MCP tool names stripped when `mcp.full` is false. Each name must match a `#[tool]` on [`TabulariumMcp`].
const MCP_FULL_ONLY_TOOL_NAMES: &[&str] = &[
    "delete_document",
    "delete_directory",
    "rename_document",
    "rename_directory",
    "move_document",
    "move_directory",
    "reindex",
];

/// Identity / ACL / PSK MCP tools — same gate as destructive ops (`mcp.full`).
const MCP_FULL_ONLY_ACL_PSK_TOOL_NAMES: &[&str] = &[
    "whoami",
    "acl_list",
    "acl_get",
    "acl_put",
    "acl_destroy",
    "psk_list",
    "psk_create",
    "psk_destroy",
];

const MCP_METHODS_CATALOG_FULL: &str = r#"

**mcp.full = true only (trusted / private deployments):**

whoami — (no params) Resolved ACL JSON for the caller.
acl_list — (no params) List ACL name + body rows (admin when auth on).
acl_get — name (string).
acl_put — name, body (validated ACL JSON string).
acl_destroy — name (string).
psk_list — (no params).
psk_create — name (PSK handle), acl_name (string). Response includes the raw key string — use it as **`X-Auth-Key`** for authenticated MCP/RPC calls.
psk_destroy — name (PSK handle).

delete_document — path (JSON-RPC delete_document).
delete_directory — path, recursive optional (default false).
rename_document — path, new_name.
rename_directory — path, new_path.
move_document — path, new_path (destination file path).
move_directory — path, new_parent, new_name.
reindex — kind optional: lexical|semantic|hybrid (default hybrid); path optional (omit or `/` for full rebuild; subtree otherwise). Semantic requires configured embeddings and the first rebuild must be full.
"#;

const MCP_METHODS_CATALOG: &str = r#"MCP tools (parameters mirror POST /rpc JSON objects).

Auth: when **`[server].authenticate = true`**, attach **`X-Auth-Key: <psk>`** on MCP HTTP requests (same as REST/`POST /rpc`), unless your proxy injects the configured OIDC assertion header. Mint keys with **`psk_create`** when **`mcp.full = true`** and admin ACL allows.

Doctrine: **list_directory** (repeated) = tree walk / "find by listing" — locate entries by path and name; **search** = hybrid lexical/semantic search across document bodies; **grep** = regex lines in one file only.

get_document_metadata — path. Read JSON metadata and revision.
set_document_metadata — path, metadata (object), only_if_revision optional. Full replacement.
help — (no params) Base doctrine + short tool index.
server_help — (no params) Text from config `server_help` path, or empty string.
methods — (no params) This catalog.

get_document — path (absolute file). Result includes `revision` (UUID string) for files.
put_document — path, content, force (default false), only_if_revision optional (UUID string). Create new document, or replace body when `force=true`. With `only_if_revision`, target must exist and revision must match — else `NotFound` or `RevisionMismatch` (-32003); overrides stale `force`. Existing target with `force=false` and no `only_if_revision` → `Duplicate` (-32002); atomic at storage layer (SQLite UNIQUE constraint).
create_document — path, content, force optional (default false), only_if_revision optional. Strict create when `force=false`; missing path + `only_if_revision` → `NotFound`. `force=true` replaces an existing file; use `only_if_revision` for guarded overwrite.
append_document — path, content, force (default false). Create new document, or append to existing body when `force=true` (`force=true` appends — it does not replace; use put_document for full replacement). Existing target with `force=false` → `Duplicate` (-32002). Not for chat/meeting blocks — use say_document.
append_if_not_contains — path, marker, content: atomically append `content` only if `marker` is **not** already a substring of the UTF-8 body (Rust `str::contains`; e.g. marker `DONE` matches inside `UNDONE`). **Document must exist** (errors if missing). Returns JSON `{ "appended": bool, "revision": string }`.
say_document — path, from_id (sender nickname), content. **Preferred for meetings, conversations, and task documents** — server appends a markdown block with the sender in the heading. **Do not prefix the nickname into content**; provide it via from_id only. **Target file must already exist** (use put_document or append_document to create). *Exempt from the `force` guard by design*: `say_document` is the only mutation operation that always appends to an existing document, because it cannot create new documents and operates exclusively on conversation/meeting contexts where appending is the intended action.
list_directory — path optional (omit or empty for root `/`). Rows include modified_at and `revision` (files only); use this to walk the tree; there is no separate MCP find tool.
search — query, path optional (subtree filter), kind optional: lexical|semantic|hybrid (default hybrid). Combines Tantivy over bodies/names/descriptions with optional semantic body excerpts; returns engine status, has_more/truncated, revision, modified_at and exact excerpt line ranges. Optional limit (1..256) and max_output_bytes (512..1048576) bound the response. sort=relevance|modified_desc; modified_after/modified_before: exclusive UNIX nanoseconds. Equal keys use canonical path.
create_directory — path, description optional, parents optional (`true` = POSIX `mkdir -p`; default `false`).
describe — path; optional description string (omit to read; empty string clears).
document_exists — path (wire RPC name `exists`; tests file only).
stat — path (includes `revision` for files).
wc — path; optional `recursive`, `lines`, `words`, `bytes`, `chars` (booleans). Count lines/words/bytes/chars like CLI `tb wc`. `path` may use a **final-segment glob** only (`/docs/*.md`, `/projects/bms/*`). Without `recursive`, glob matches **files directly in that folder** (not subfolders). With `recursive: true`, each **matched directory** is walked for all files beneath it; a **bare directory path** also requires `recursive`. Omit metric flags → all four fields; set flags to select columns (`bytes` = UTF-8 length, POSIX `wc -c`; `chars` = Unicode scalars). Response: **one resolved file** → flat `{ bytes, lines, words, chars }` (subset when flags set); **two or more files** → `{ "files": [{ "path", …counts }], "total": { …counts } }`. Errors: bare directory without `recursive` → `wc: PATH: is a directory`; glob matches only subdirectories without `recursive` → hint to use `recursive`.
  Examples (request → shape):
  • One file, all metrics: `{ "path": "/docs/readme.md" }` → `{ "bytes", "lines", "words", "chars" }`
  • Lines only: `{ "path": "/docs/readme.md", "lines": true }` → `{ "lines": 42 }`
  • Markdown in one folder (non-recursive): `{ "path": "/docs/*.md", "lines": true }` → `{ "files": [{ "path", "lines" }, …], "total": { "lines" } }`
  • Project tree via glob on subdirs: `{ "path": "/projects/bms/*", "recursive": true, "lines": true }` → all files under each immediate child of `bms`
  • Whole subtree: `{ "path": "/projects/bms", "recursive": true, "bytes": true }` → every file under `bms`
head — path, lines optional (default 10 like GNU head); number or string integer (`0` = zero lines, not unlimited).
tail — path, lines optional (default 10 last lines like GNU tail); number, string integer (`0` = zero lines), or "+N" from-line form.
slice — path, start_line/end_line or from_line/to_line aliases (1-based inclusive); numbers or string integers.
grep — path, pattern, max_matches optional (0 = unlimited), invert_match optional (default false). Line-level regex within that single document; not repo-wide search.
wait — path (long-poll until document changes or server timeout).

When `mcp.full = true` in server config, destructive tools plus identity / ACL / PSK admin tools (`whoami`, `acl_*`, `psk_*`) are also registered — see suffix in tool `methods` output. Otherwise use CLI or REST for those operations.
"#;

#[derive(Clone)]
pub struct TabulariumMcp {
    app: AppState,
    server_help: Arc<str>,
    mcp_full: bool,
    tool_router: ToolRouter<Self>,
}

fn tail_lines_to_rpc_value(lines: Option<&Value>) -> std::result::Result<Value, String> {
    let Some(v) = lines else {
        return Ok(json!(10));
    };
    if v.is_null() {
        return Ok(json!(10));
    }
    match v {
        Value::Number(n) => {
            let u = n
                .as_u64()
                .ok_or_else(|| "lines: number out of range".to_string())?;
            let u = u32::try_from(u).map_err(|_| "lines out of range".to_string())?;
            Ok(json!(u))
        }
        Value::String(s) => {
            let t = s.trim();
            if t.starts_with('+') {
                Ok(json!(t))
            } else {
                let u: u32 = t
                    .parse()
                    .map_err(|_| format!("lines: expected integer or '+N' string, got {t:?}"))?;
                Ok(json!(u))
            }
        }
        _ => Err("lines: expected number, string, or null".into()),
    }
}

impl TabulariumMcp {
    pub fn new(app: AppState, server_help: Arc<str>, mcp_full: bool) -> Self {
        let mut tool_router = Self::tool_router();
        if !mcp_full {
            for name in MCP_FULL_ONLY_TOOL_NAMES {
                tool_router.remove_route(name);
            }
            for name in MCP_FULL_ONLY_ACL_PSK_TOOL_NAMES {
                tool_router.remove_route(name);
            }
        }
        Self {
            app,
            server_help,
            mcp_full,
            tool_router,
        }
    }

    /// Whether an MCP tool with this name is registered (depends on `mcp.full`).
    pub fn has_mcp_tool(&self, name: &str) -> bool {
        self.tool_router.has_route(name)
    }

    async fn call_rpc_json(
        &self,
        parts: &Parts,
        method: &str,
        params: Value,
    ) -> Result<String, String> {
        let span = info_span!(
            "mcp.tools/call",
            otel.name = "mcp.tools/call",
            mcp.method = method,
            rpc.method = method,
            mcp.tool = method,
            otel.status_code = field::Empty,
            error.type = field::Empty,
        );
        async {
            let _active = enter_active_tool(method);
            let started = Instant::now();
            let mcp_full_only_tool = MCP_FULL_ONLY_TOOL_NAMES.contains(&method)
                || MCP_FULL_ONLY_ACL_PSK_TOOL_NAMES.contains(&method);
            info!(
                target: "tabularium_server::rpc",
                transport = "mcp",
                mcp_full = self.mcp_full,
                mcp_full_only_tool,
                method = %method,
                params = %crate::rpc_preview::format_rpc_params_preview(Some(&params)),
                "RPC request"
            );
            let auth: RequestAuth = if !self.app.authenticate_mcp {
                RequestAuth::Disabled
            } else if method == "test" {
                RequestAuth::Disabled
            } else {
                match resolve_request_auth_arc(
                    self.app.db.as_ref(),
                    true,
                    &parts.headers,
                    self.app.oidc.as_ref(),
                    self.app.anonymous.as_ref(),
                )
                .await
                {
                    Ok(a) => a,
                    Err(e) => {
                        let msg = e.to_string();
                        tracing::Span::current().record("otel.status_code", "ERROR");
                        tracing::Span::current().record("error.type", "auth_error");
                        record_tool_call(method, started, false);
                        return Err(msg);
                    }
                }
            };
            let map = params.as_object().cloned().unwrap_or_default();
            let result = match dispatch_app_rpc(&self.app, &auth, method, map).await {
                Ok(v) => Ok(if v.is_null() {
                    "null".to_string()
                } else {
                    serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?
                }),
                Err(e) => Err(match e {
                    RpcAppError::MethodNotFound(m) => m,
                    RpcAppError::Other(err) => err.to_string(),
                }),
            };
            match &result {
                Ok(_) => {
                    tracing::Span::current().record("otel.status_code", "OK");
                    record_tool_call(method, started, true);
                }
                Err(_) => {
                    tracing::Span::current().record("otel.status_code", "ERROR");
                    tracing::Span::current().record("error.type", "tool_error");
                    record_tool_call(method, started, false);
                }
            }
            result
        }
        .instrument(span)
        .await
    }
}

#[derive(Deserialize, Default)]
struct Empty {}

impl JsonSchema for Empty {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Empty".into()
    }
    fn json_schema(_gen: &mut schemars::generate::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"type": "object", "properties": {}})
    }
}

#[derive(Deserialize, JsonSchema)]
struct PathArg {
    path: String,
}

#[derive(Deserialize, JsonSchema)]
struct PathContentCreate {
    path: String,
    content: String,
    /// Overwrite existing target when `true`; optional compare-and-swap via `only_if_revision`.
    #[serde(default)]
    force: bool,
    /// When set, asserts the path names an existing file whose current revision matches this UUID string.
    #[serde(default)]
    only_if_revision: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct PathContentForce {
    path: String,
    content: String,
    /// Required `true` to mutate an existing document.
    /// Default `false`: existing target → `Duplicate` (-32002); use this safe default for agents.
    /// `true`: legacy upsert semantics (`put_document` replaces body, `append_document` appends bytes).
    #[serde(default)]
    force: bool,
    /// Compare-and-swap: replace only if the stored revision matches (missing file → `NotFound`).
    #[serde(default)]
    only_if_revision: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct PathMarkerContent {
    path: String,
    marker: String,
    content: String,
}

#[derive(Deserialize, JsonSchema)]
struct SayArg {
    path: String,
    from_id: String,
    content: String,
}

#[derive(Deserialize, JsonSchema, Default)]
struct ListDirectoryArg {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum KindArg {
    Lexical,
    Semantic,
    Hybrid,
}
impl KindArg {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Lexical => "lexical",
            Self::Semantic => "semantic",
            Self::Hybrid => "hybrid",
        }
    }
}

#[derive(Deserialize, JsonSchema)]
struct SearchArg {
    query: String,
    #[serde(default)]
    path: Option<String>,
    /// Engine selection; defaults to hybrid.
    #[serde(default)]
    kind: Option<KindArg>,
    /// Maximum results, 1..256 (default 256).
    #[serde(default)]
    limit: Option<usize>,
    /// Compact UTF-8 JSON payload budget, 512..1048576 (default 65536).
    #[serde(default)]
    max_output_bytes: Option<usize>,
    /// relevance (default) or modified_desc; ties use canonical path.
    #[serde(default)]
    sort: Option<String>,
    /// Exclusive lower bound, UNIX nanoseconds (same unit as modified_at).
    #[serde(default)]
    modified_after: Option<u64>,
    /// Exclusive upper bound, UNIX nanoseconds.
    #[serde(default)]
    modified_before: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
struct CreateDirectoryArg {
    path: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    parents: bool,
}

#[derive(Deserialize, JsonSchema)]
struct DescribeArg {
    path: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
enum LooseNat {
    Num(u32),
    Str(String),
}

impl LooseNat {
    fn parse_u32(self) -> std::result::Result<u32, String> {
        match self {
            LooseNat::Num(n) => Ok(n),
            LooseNat::Str(s) => s
                .trim()
                .parse()
                .map_err(|_| format!("expected non-negative integer string, got {:?}", s.trim())),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
struct HeadArg {
    path: String,
    #[serde(default)]
    lines: Option<LooseNat>,
}

#[derive(Deserialize, JsonSchema)]
struct TailArg {
    path: String,
    /// Omitted = GNU default (10). Number, decimal string, or `+N` tail-from-line string.
    #[serde(default)]
    lines: Option<Value>,
}

#[derive(Deserialize, JsonSchema)]
struct SliceArg {
    path: String,
    #[serde(alias = "from_line")]
    start_line: LooseNat,
    #[serde(alias = "to_line")]
    end_line: LooseNat,
    /// Assert the revision returned by search; mismatch returns RevisionMismatch.
    #[serde(default)]
    expected_revision: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct GrepArg {
    path: String,
    pattern: String,
    #[serde(default)]
    max_matches: Option<u64>,
    #[serde(default)]
    invert_match: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct WcArg {
    path: String,
    #[serde(default)]
    recursive: Option<bool>,
    #[serde(default)]
    lines: Option<bool>,
    #[serde(default)]
    words: Option<bool>,
    #[serde(default)]
    bytes: Option<bool>,
    #[serde(default)]
    chars: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct DeleteDirectoryArg {
    path: String,
    #[serde(default)]
    recursive: bool,
}

#[derive(Deserialize, JsonSchema)]
struct RenameDocumentArg {
    path: String,
    new_name: String,
}

#[derive(Deserialize, JsonSchema)]
struct RenameDirectoryArg {
    path: String,
    new_path: String,
}

#[derive(Deserialize, JsonSchema)]
struct MoveDocumentArg {
    path: String,
    new_path: String,
}

#[derive(Deserialize, JsonSchema)]
struct MoveDirectoryArg {
    path: String,
    new_parent: String,
    new_name: String,
}

#[derive(Deserialize, JsonSchema, Default)]
struct ReindexArg {
    #[serde(default)]
    path: Option<String>,
    /// Engine selection; defaults to hybrid.
    #[serde(default)]
    kind: Option<KindArg>,
}

#[derive(Deserialize, JsonSchema)]
struct AclNameArg {
    name: String,
}

#[derive(Deserialize, JsonSchema)]
struct AclPutArg {
    name: String,
    body: String,
}

#[derive(Deserialize, JsonSchema)]
struct PskCreateArg {
    name: String,
    acl_name: String,
}

#[derive(Deserialize, JsonSchema)]
struct PskDestroyArg {
    name: String,
}

#[tool_router]
impl TabulariumMcp {
    #[tool(
        description = "Base help: Tabularium as filesystem-like shared document store for AI agents, plus a one-line tool index. For full parameter shapes call `methods`."
    )]
    async fn help(
        &self,
        Extension(_parts): Extension<Parts>,
        Parameters(_): Parameters<Empty>,
    ) -> String {
        let mut t = MCP_HELP_BASE.to_string();
        t.push_str("\n\n");
        t.push_str(TOOL_NAMES_LINE);
        t
    }

    #[tool(
        name = "server_help",
        description = "Optional deployment help text from config `server_help` path; empty string when unset."
    )]
    async fn server_help_tool(
        &self,
        Extension(_parts): Extension<Parts>,
        Parameters(_): Parameters<Empty>,
    ) -> String {
        self.server_help.to_string()
    }

    #[tool(
        description = "Terse catalog of MCP tools and JSON parameter fields (matches JSON-RPC where applicable)."
    )]
    async fn methods(
        &self,
        Extension(_parts): Extension<Parts>,
        Parameters(_): Parameters<Empty>,
    ) -> String {
        let mut s = MCP_METHODS_CATALOG.to_string();
        if self.mcp_full {
            s.push_str(MCP_METHODS_CATALOG_FULL);
        }
        s
    }

    #[tool(description = "Read arbitrary JSON document metadata and shared revision.")]
    async fn get_document_metadata(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<PathArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(&parts, "get_document_metadata", json!({"path":p.path}))
            .await
    }

    #[tool(
        description = "Replace the document JSON metadata object; optional only_if_revision protects against concurrent edits. Metadata is lexically indexed."
    )]
    async fn set_document_metadata(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<MetadataArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(
            &parts,
            "set_document_metadata",
            json!({"path":p.path,"metadata":p.metadata,"only_if_revision":p.only_if_revision}),
        )
        .await
    }

    #[tool(description = "Read full document body and metadata (JSON-RPC get_document).")]
    async fn get_document(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<PathArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(&parts, "get_document", json!({ "path": p.path }))
            .await
    }

    #[tool(
        description = "Write `content` to `path` (JSON-RPC put_document). **Default safe**: errors with `Duplicate` (-32002) if the document already exists; pass `force=true` to deliberately overwrite an existing body. Missing target is always created. The check-and-create is atomic at the storage layer (SQLite UNIQUE constraint), so concurrent callers cannot both win when `force=false`."
    )]
    async fn put_document(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<PathContentForce>,
    ) -> Result<String, String> {
        self.call_rpc_json(
            &parts,
            "put_document",
            json!({
                "path": p.path,
                "content": p.content,
                "force": p.force,
                "only_if_revision": p.only_if_revision,
            }),
        )
        .await
    }

    #[tool(
        description = "Create a new file (JSON-RPC create_document). Optional force + only_if_revision for guarded overwrite."
    )]
    async fn create_document(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<PathContentCreate>,
    ) -> Result<String, String> {
        self.call_rpc_json(
            &parts,
            "create_document",
            json!({
                "path": p.path,
                "content": p.content,
                "force": p.force,
                "only_if_revision": p.only_if_revision,
            }),
        )
        .await
    }

    #[tool(
        description = "Append bytes to document (JSON-RPC append_document). Not for chat/meeting lines — use say_document so from_id is recorded. **Default safe**: errors with `Duplicate` (-32002) if the document already exists; pass `force=true` to deliberately append to an existing body. Missing target is always created. With `force=true` the call appends — it does not replace; use put_document for full replacement."
    )]
    async fn append_document(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<PathContentForce>,
    ) -> Result<String, String> {
        self.call_rpc_json(
            &parts,
            "append_document",
            json!({ "path": p.path, "content": p.content, "force": p.force }),
        )
        .await
    }

    #[tool(
        description = "Append `content` only if `marker` is not already a substring of the document body (JSON-RPC append_if_not_contains). Per-document lock; document must exist; returns JSON boolean true when appended, false when skipped."
    )]
    async fn append_if_not_contains(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<PathMarkerContent>,
    ) -> Result<String, String> {
        self.call_rpc_json(
            &parts,
            "append_if_not_contains",
            json!({
                "path": p.path,
                "marker": p.marker,
                "content": p.content,
            }),
        )
        .await
    }

    #[tool(
        description = "Append a markdown chat block; **preferred for meetings and conversations** — `from_id` is the sender nickname recorded in the appended block. Do not include your nickname in `content` (JSON-RPC say_document)."
    )]
    async fn say_document(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<SayArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(
            &parts,
            "say_document",
            json!({ "path": p.path, "from_id": p.from_id, "content": p.content }),
        )
        .await
    }

    #[tool(
        description = "List entries under a directory (JSON-RPC list_directory); each row includes modified_at. Tree-walk / \"find by listing\" — there is no separate MCP find tool."
    )]
    async fn list_directory(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<ListDirectoryArg>,
    ) -> Result<String, String> {
        let mut m = Map::new();
        if let Some(ref path) = p.path
            && !path.is_empty()
        {
            m.insert("path".into(), json!(path));
        }
        self.call_rpc_json(&parts, "list_directory", Value::Object(m))
            .await
    }

    #[tool(
        description = "Hybrid search with exact source excerpts, revision, modified_at and engine status. Optional limit and max_output_bytes bound the payload; has_more/truncated explicitly mark omitted hits. Optional kind=lexical|semantic|hybrid (default hybrid); sort=relevance|modified_desc. modified_after/before are exclusive UNIX nanoseconds. Ties use canonical path. path limits subtree or file. For path/name listing use list_directory; for line-regex use grep."
    )]
    async fn search(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<SearchArg>,
    ) -> Result<String, String> {
        let mut m = Map::new();
        m.insert("with_status".into(), json!(true));
        if let Some(n) = p.limit {
            m.insert("limit".into(), json!(n));
        }
        if let Some(n) = p.max_output_bytes {
            m.insert("max_output_bytes".into(), json!(n));
        }
        if let Some(kind) = p.kind {
            m.insert("kind".into(), json!(kind.as_str()));
        }
        if let Some(sort) = p.sort {
            m.insert("sort".into(), json!(sort));
        }
        if let Some(n) = p.modified_after {
            m.insert("modified_after".into(), json!(n));
        }
        if let Some(n) = p.modified_before {
            m.insert("modified_before".into(), json!(n));
        }
        m.insert("query".into(), json!(p.query));
        if let Some(path) = p.path.filter(|s| !s.is_empty()) {
            m.insert("path".into(), json!(path));
        }
        self.call_rpc_json(&parts, "search", Value::Object(m)).await
    }

    #[tool(
        description = "Create a directory (JSON-RPC create_directory). Optional parents=true creates missing ancestors (mkdir -p)."
    )]
    async fn create_directory(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<CreateDirectoryArg>,
    ) -> Result<String, String> {
        let mut m = Map::new();
        m.insert("path".into(), json!(p.path));
        if let Some(d) = p.description {
            m.insert("description".into(), json!(d));
        }
        if p.parents {
            m.insert("parents".into(), json!(true));
        }
        self.call_rpc_json(&parts, "create_directory", Value::Object(m))
            .await
    }

    #[tool(
        description = "Get or set `description` on a file or directory (JSON-RPC describe). Omit `description` to read; pass a string to set; empty string clears."
    )]
    async fn describe(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<DescribeArg>,
    ) -> Result<String, String> {
        let mut m = Map::new();
        m.insert("path".into(), json!(p.path));
        if let Some(d) = p.description {
            m.insert("description".into(), json!(d));
        }
        self.call_rpc_json(&parts, "describe", Value::Object(m))
            .await
    }

    #[tool(description = "Whether path is an existing file (JSON-RPC exists).")]
    async fn document_exists(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<PathArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(&parts, "exists", json!({ "path": p.path }))
            .await
    }

    #[tool(description = "File metadata and line count (JSON-RPC stat).")]
    async fn stat(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<PathArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(&parts, "stat", json!({ "path": p.path }))
            .await
    }

    #[tool(
        description = r#"Line/word/byte/char counts (JSON-RPC wc). Like `tb wc -l/-w/-c/-m` and `-r`.

Params: `path` (required); optional booleans `recursive`, `lines`, `words`, `bytes`, `chars`. Final-segment glob only (`/docs/*.md`, `/projects/bms/*`). Without `recursive`, glob counts files in one directory level. With `recursive`, matched directories and bare directory paths walk full subtrees. Omit metric flags for all four counts.

Response: single file → flat object; two or more files → `{ "files": [{ "path", … }], "total": { … } }`.

Examples:
• `{ "path": "/docs/readme.md" }` — all metrics on one file
• `{ "path": "/docs/readme.md", "lines": true }` — lines only
• `{ "path": "/docs/*.md", "lines": true }` — each `.md` in `/docs` + total
• `{ "path": "/projects/bms/*", "recursive": true, "lines": true }` — lines in every file under each child of `bms` (use when the folder contains only subdirectories)
• `{ "path": "/projects/bms", "recursive": true, "bytes": true }` — UTF-8 bytes for the whole `bms` tree"#
    )]
    async fn wc(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<WcArg>,
    ) -> Result<String, String> {
        let mut m = Map::new();
        m.insert("path".into(), json!(p.path));
        if let Some(v) = p.recursive {
            m.insert("recursive".into(), json!(v));
        }
        if let Some(v) = p.lines {
            m.insert("lines".into(), json!(v));
        }
        if let Some(v) = p.words {
            m.insert("words".into(), json!(v));
        }
        if let Some(v) = p.bytes {
            m.insert("bytes".into(), json!(v));
        }
        if let Some(v) = p.chars {
            m.insert("chars".into(), json!(v));
        }
        self.call_rpc_json(&parts, "wc", Value::Object(m)).await
    }

    #[tool(
        description = "First N logical lines (JSON-RPC head). `lines` optional (default 10); number or string integer. `0` returns no lines."
    )]
    async fn head(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<HeadArg>,
    ) -> Result<String, String> {
        let mut m = Map::new();
        m.insert("path".into(), json!(p.path));
        if let Some(l) = p.lines {
            m.insert(
                "lines".into(),
                json!(l.parse_u32().map_err(|e| e.to_string())?),
            );
        }
        self.call_rpc_json(&parts, "head", Value::Object(m)).await
    }

    #[tool(
        description = "Tail of file (JSON-RPC tail). `lines` optional (default 10 last lines); number, string integer (`0` = no lines), or '+N' from-line."
    )]
    async fn tail(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<TailArg>,
    ) -> Result<String, String> {
        let mut m = Map::new();
        m.insert("path".into(), json!(p.path));
        m.insert(
            "lines".into(),
            tail_lines_to_rpc_value(p.lines.as_ref()).map_err(|e| e.to_string())?,
        );
        self.call_rpc_json(&parts, "tail", Value::Object(m)).await
    }

    #[tool(
        description = "Inclusive 1-based line range (JSON-RPC slice). Optional expected_revision detects source changes; returns text and revision. `start_line`/`end_line` or `from_line`/`to_line`; number or string integer."
    )]
    async fn slice(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<SliceArg>,
    ) -> Result<String, String> {
        let start = p.start_line.parse_u32().map_err(|e| e.to_string())?;
        let end = p.end_line.parse_u32().map_err(|e| e.to_string())?;
        self.call_rpc_json(
            &parts,
            "slice",
            json!({
                "path": p.path,
                "expected_revision": p.expected_revision,
                "start_line": start,
                "end_line": end,
            }),
        )
        .await
    }

    #[tool(
        description = "Regex line matches within one document (JSON-RPC grep); max_matches 0 = unlimited. Not indexed repo-wide search — use search; not tree listing — use list_directory."
    )]
    async fn grep(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<GrepArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(
            &parts,
            "grep",
            json!({
                "path": p.path,
                "pattern": p.pattern,
                "max_matches": p.max_matches.unwrap_or(0),
                "invert_match": p.invert_match.unwrap_or(false),
            }),
        )
        .await
    }

    #[tool(description = "Long-poll until document changes or timeout (JSON-RPC wait).")]
    async fn wait(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<PathArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(&parts, "wait", json!({ "path": p.path }))
            .await
    }

    #[tool(
        description = "Resolved ACL identity JSON (`whoami` RPC). **Registered only when `mcp.full = true` in server config.** Valid `X-Auth-Key` or assertion header required when `[mcp].authenticate=true`."
    )]
    async fn whoami(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(_): Parameters<Empty>,
    ) -> Result<String, String> {
        self.call_rpc_json(&parts, "whoami", json!({})).await
    }

    #[tool(
        description = "List ACL definitions (`acl_list`). **Registered only when `mcp.full = true`.** Admin ACL required when `[mcp].authenticate=true`."
    )]
    async fn acl_list(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(_): Parameters<Empty>,
    ) -> Result<String, String> {
        self.call_rpc_json(&parts, "acl_list", json!({})).await
    }

    #[tool(
        description = "Fetch one ACL body JSON (`acl_get`). **Registered only when `mcp.full = true`.** Admin when auth on."
    )]
    async fn acl_get(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<AclNameArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(&parts, "acl_get", json!({ "name": p.name }))
            .await
    }

    #[tool(
        description = "Upsert ACL (`acl_put`); body must be valid ACL JSON. **Registered only when `mcp.full = true`.** Admin when auth on."
    )]
    async fn acl_put(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<AclPutArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(&parts, "acl_put", json!({ "name": p.name, "body": p.body }))
            .await
    }

    #[tool(
        description = "Delete ACL and cascade keys (`acl_destroy`). **Registered only when `mcp.full = true`.** Admin when auth on."
    )]
    async fn acl_destroy(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<AclNameArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(&parts, "acl_destroy", json!({ "name": p.name }))
            .await
    }

    #[tool(
        description = "List PSK rows (`psk_list`). **Registered only when `mcp.full = true`.** Admin when auth on. Requires `X-Auth-Key` or assertion when `[mcp].authenticate=true`."
    )]
    async fn psk_list(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(_): Parameters<Empty>,
    ) -> Result<String, String> {
        self.call_rpc_json(&parts, "psk_list", json!({})).await
    }

    #[tool(
        description = "Create PSK for ACL (`psk_create`). **Registered only when `mcp.full = true`.** Admin when auth on. Returns the new key — store as `X-Auth-Key` for subsequent MCP/RPC requests when server auth is enabled."
    )]
    async fn psk_create(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<PskCreateArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(
            &parts,
            "psk_create",
            json!({ "name": p.name, "acl_name": p.acl_name }),
        )
        .await
    }

    #[tool(
        description = "Delete PSK (`psk_destroy`). **Registered only when `mcp.full = true`.** Admin when auth on. Requires `X-Auth-Key` or assertion when `[mcp].authenticate=true`."
    )]
    async fn psk_destroy(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<PskDestroyArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(&parts, "psk_destroy", json!({ "name": p.name }))
            .await
    }

    #[tool(
        description = "Delete a file (JSON-RPC delete_document). **Registered only when `mcp.full = true` in server config.**"
    )]
    async fn delete_document(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<PathArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(&parts, "delete_document", json!({ "path": p.path }))
            .await
    }

    #[tool(
        description = "Delete a directory (JSON-RPC delete_directory). **Registered only when `mcp.full = true`.**"
    )]
    async fn delete_directory(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<DeleteDirectoryArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(
            &parts,
            "delete_directory",
            json!({ "path": p.path, "recursive": p.recursive }),
        )
        .await
    }

    #[tool(
        description = "Rename a file's last segment (JSON-RPC rename_document). **Registered only when `mcp.full = true`.**"
    )]
    async fn rename_document(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<RenameDocumentArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(
            &parts,
            "rename_document",
            json!({ "path": p.path, "new_name": p.new_name }),
        )
        .await
    }

    #[tool(
        description = "Rename/move a directory by full new_path (JSON-RPC rename_directory). **Registered only when `mcp.full = true`.**"
    )]
    async fn rename_directory(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<RenameDirectoryArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(
            &parts,
            "rename_directory",
            json!({ "path": p.path, "new_path": p.new_path }),
        )
        .await
    }

    #[tool(
        description = "Move a file to a new absolute file path (JSON-RPC move_document). **Registered only when `mcp.full = true`.**"
    )]
    async fn move_document(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<MoveDocumentArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(
            &parts,
            "move_document",
            json!({ "path": p.path, "new_path": p.new_path }),
        )
        .await
    }

    #[tool(
        description = "Move a directory under a new parent with a new leaf name (JSON-RPC move_directory). **Registered only when `mcp.full = true`.**"
    )]
    async fn move_directory(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<MoveDirectoryArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(
            &parts,
            "move_directory",
            json!({
                "path": p.path,
                "new_parent": p.new_parent,
                "new_name": p.new_name,
            }),
        )
        .await
    }

    #[tool(
        description = "Rebuild selected search indexes (kind=lexical|semantic|hybrid, default hybrid); optional path limits subtree. **Registered only when `mcp.full = true`.**"
    )]
    async fn reindex(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(p): Parameters<ReindexArg>,
    ) -> Result<String, String> {
        let mut m = Map::new();
        if let Some(kind) = p.kind {
            m.insert("kind".into(), json!(kind.as_str()));
        }
        if let Some(path) = p.path.filter(|s| !s.is_empty()) {
            m.insert("path".into(), json!(path));
        }
        self.call_rpc_json(&parts, "reindex", Value::Object(m))
            .await
    }
}

#[tool_handler]
impl ServerHandler for TabulariumMcp {
    fn get_info(&self) -> ServerInfo {
        let instructions = if self.mcp_full {
            "Tabularium MCP — **trusted FULL mode**. Same JSON-RPC semantics and authentication as POST /rpc (no alternate permission model). Includes destructive tools, identity (`whoami`), and ACL/PSK admin tools — plus the standard document operations. **Private / operator-controlled deployments only.**"
        } else {
            "Tabularium MCP — filesystem-like document store for AI agents. Three operations: list_directory (tree walk / find-by-listing, rows carry modified_at), search (hybrid lexical/semantic across bodies, file names, descriptions), grep (regex lines in one file). say_document for meetings (`from_id` = sender nickname). append_document is not for chat blocks. Destructive tools and ACL/PSK/`whoami` MCP tools are not registered unless `mcp.full = true` in server config; otherwise use CLI or REST."
        };
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(instructions)
    }
}

impl TabulariumMcp {
    /// In-process MCP→RPC invocation for integration tests (empty request parts, no auth headers).
    #[doc(hidden)]
    pub async fn invoke_rpc_for_test(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<String, String> {
        let req = http::Request::builder()
            .method("POST")
            .uri("/mcp")
            .body(())
            .expect("test request");
        let (parts, ()) = req.into_parts();
        self.call_rpc_json(&parts, method, params).await
    }
}

/// Streamable HTTP MCP endpoint at `http://{listen}/mcp`.
pub async fn serve(
    listen: &str,
    app: AppState,
    server_help: String,
    mcp_full: bool,
    cancel: CancellationToken,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(listen).await?;
    let help: Arc<str> = Arc::from(server_help);
    let template = TabulariumMcp::new(app, Arc::clone(&help), mcp_full);
    let service = StreamableHttpService::new(
        move || Ok(template.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default().with_cancellation_token(cancel),
    );
    let router = Router::new().nest_service("/mcp", service);
    info!(
        listen = %listen,
        "MCP streamable HTTP listening at http://{}/mcp",
        listen
    );
    axum::serve(listener, router).await?;
    Ok(())
}

#[cfg(test)]
mod search_option_tests {
    use super::*;
    use bma_ts::Monotonic;
    use std::time::Duration;
    use tabularium::SqliteDatabase;

    #[tokio::test]
    async fn mcp_metadata_roundtrip_and_revision_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(
            SqliteDatabase::init(
                format!("sqlite://{}", dir.path().join("source.db").display()),
                dir.path().join("index"),
                0,
            )
            .await
            .unwrap(),
        );
        db.create_file_in_directory("/", "example", "body")
            .await
            .unwrap();
        let mcp = TabulariumMcp::new(
            AppState {
                db,
                wait_timeout: Duration::from_secs(1),
                process_started_at: Monotonic::now(),
                authenticate_api: false,
                authenticate_mcp: false,
                oidc: None,
            },
            Arc::from(""),
            false,
        );
        let parts = || http::Request::new(()).into_parts().0;
        let read = || {
            Parameters(PathArg {
                path: "/example".into(),
            })
        };
        let value: Value = serde_json::from_str(
            &mcp.get_document_metadata(Extension(parts()), read())
                .await
                .unwrap(),
        )
        .unwrap();
        let revision = value["revision"].as_str().unwrap().to_string();
        let write = || {
            Parameters(MetadataArg {
                path: "/example".into(),
                metadata: json!({"tags":["test"]}),
                only_if_revision: Some(revision.clone()),
            })
        };
        mcp.set_document_metadata(Extension(parts()), write())
            .await
            .unwrap();
        assert!(
            mcp.set_document_metadata(Extension(parts()), write())
                .await
                .is_err()
        );
        let value: Value = serde_json::from_str(
            &mcp.get_document_metadata(Extension(parts()), read())
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(value["metadata"], json!({"tags":["test"]}));
    }

    #[tokio::test]
    async fn mcp_search_forwards_sort_time_bounds_and_validates_sort() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(
            SqliteDatabase::init(
                format!("sqlite://{}", dir.path().join("source.db").display()),
                dir.path().join("index"),
                0,
            )
            .await
            .unwrap(),
        );
        for name in ["a", "b"] {
            db.create_file_in_directory("/", name, "needle")
                .await
                .unwrap();
        }
        let mcp = TabulariumMcp::new(
            AppState {
                db,
                wait_timeout: Duration::from_secs(1),
                process_started_at: Monotonic::now(),
                authenticate_api: false,
                authenticate_mcp: false,
                oidc: None,
            },
            Arc::from(""),
            false,
        );
        let parts = || http::Request::new(()).into_parts().0;
        let args = |extra: Value| {
            let mut value =
                json!({"query":"needle","kind":"lexical","limit":1,"sort":"modified_desc"});
            value
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            serde_json::from_value::<SearchArg>(value).unwrap()
        };
        let result = mcp
            .search(Extension(parts()), Parameters(args(json!({}))))
            .await
            .unwrap();
        let report: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(report["hits"][0]["path"], "/b");
        assert_eq!(report["has_more"], true);
        let timestamp = report["hits"][0]["modified_at"].as_u64().unwrap();
        let result = mcp
            .search(
                Extension(parts()),
                Parameters(args(json!({"modified_after":timestamp}))),
            )
            .await
            .unwrap();
        let report: Value = serde_json::from_str(&result).unwrap();
        assert!(report["hits"].as_array().unwrap().is_empty());
        assert_eq!(report["has_more"], false);
        assert!(
            mcp.search(
                Extension(parts()),
                Parameters(args(json!({"sort":"invalid"})))
            )
            .await
            .is_err()
        );
    }
}

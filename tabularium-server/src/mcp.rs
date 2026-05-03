//! MCP streamable HTTP (`/mcp`) — transport glue to the same JSON-RPC semantics as [`crate::web::dispatch_app_rpc`].

use std::sync::Arc;

use axum::Router;
use rmcp::{
    ServerHandler,
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
use tracing::info;

use crate::web::{AppState, RpcAppError, dispatch_app_rpc};

const MCP_HELP_BASE: &str = include_str!("../res/mcp/help.txt");

const TOOL_NAMES_LINE: &str = "Call tool `methods` for the live catalog; the registered set depends on config `mcp.full` (destructive tools are omitted unless full mode is enabled).";

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

const MCP_METHODS_CATALOG_FULL: &str = r#"

**mcp.full = true only (trusted / private deployments):**

delete_document — path (JSON-RPC delete_document).
delete_directory — path, recursive optional (default false).
rename_document — path, new_name.
rename_directory — path, new_path.
move_document — path, new_path (destination file path).
move_directory — path, new_parent, new_name.
reindex — path optional (omit or `/` for full rebuild; subtree otherwise).
"#;

const MCP_METHODS_CATALOG: &str = r#"MCP tools (parameters mirror POST /rpc JSON objects).

Doctrine: **list_directory** (repeated) = tree walk / "find by listing" — locate entries by path and name; **search** = indexed full-text across document bodies; **grep** = regex lines in one file only.

help — (no params) Base doctrine + short tool index.
server_help — (no params) Text from config `server_help` path, or empty string.
methods — (no params) This catalog.

get_document — path (absolute file).
put_document — path, content (create or replace full body).
create_document — path, content (new file; parent directory must exist).
append_document — path, content (raw append; not for chat/meeting blocks — use say_document).
say_document — path, from_id (sender nickname), content. **Preferred for meetings, conversations, and task scrolls** — server appends a markdown block with the sender in the heading. **Do not prefix the nickname into content**; provide it via from_id only. **Target file must already exist** (use put_document or append_document to create).
list_directory — path optional (omit or empty for root `/`). Rows include modified_at; use this to walk the tree; there is no separate MCP find tool.
search — query, path optional (subtree filter). Indexed full-text over document body, file name, and description.
create_directory — path, description optional, parents optional (`true` = POSIX `mkdir -p`; default `false`).
describe — path; optional description string (omit to read; empty string clears).
document_exists — path (wire RPC name `exists`; tests file only).
stat — path.
wc — path.
head — path, lines optional (default 10 like GNU head); number or string integer (`0` = zero lines, not unlimited).
tail — path, lines optional (default 10 last lines like GNU tail); number, string integer (`0` = zero lines), or "+N" from-line form.
slice — path, start_line/end_line or from_line/to_line aliases (1-based inclusive); numbers or string integers.
grep — path, pattern, max_matches optional (0 = unlimited), invert_match optional (default false). Line-level regex within that single document; not repo-wide search.
wait — path (long-poll until document changes or server timeout).

When `mcp.full = true` in server config, destructive tools are also registered — see suffix in tool `methods` output.
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

    async fn call_rpc_json(&self, method: &str, params: Value) -> Result<String, String> {
        let mcp_destructive = MCP_FULL_ONLY_TOOL_NAMES.contains(&method);
        info!(
            target: "tabularium_server::rpc",
            transport = "mcp",
            mcp_full = self.mcp_full,
            mcp_destructive,
            method = %method,
            params = %crate::rpc_preview::format_rpc_params_preview(Some(&params)),
            "RPC request"
        );
        let map = params.as_object().cloned().unwrap_or_default();
        match dispatch_app_rpc(&self.app, method, map).await {
            Ok(v) => Ok(if v.is_null() {
                "null".to_string()
            } else {
                serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?
            }),
            Err(e) => Err(match e {
                RpcAppError::MethodNotFound(m) => m,
                RpcAppError::Other(err) => err.to_string(),
            }),
        }
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
struct PathContent {
    path: String,
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
struct SearchArg {
    query: String,
    #[serde(default)]
    path: Option<String>,
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
}

#[tool_router]
impl TabulariumMcp {
    #[tool(
        description = "Base help: Tabularium as fs-like shared librarium for spirits, plus a one-line tool index. For full parameter shapes call `methods`."
    )]
    async fn help(&self, Parameters(_): Parameters<Empty>) -> String {
        let mut t = MCP_HELP_BASE.to_string();
        t.push_str("\n\n");
        t.push_str(TOOL_NAMES_LINE);
        t
    }

    #[tool(
        name = "server_help",
        description = "Optional deployment help text from config `server_help` path; empty string when unset."
    )]
    async fn server_help_tool(&self, Parameters(_): Parameters<Empty>) -> String {
        self.server_help.to_string()
    }

    #[tool(
        description = "Terse catalog of MCP tools and JSON parameter fields (matches JSON-RPC where applicable)."
    )]
    async fn methods(&self, Parameters(_): Parameters<Empty>) -> String {
        let mut s = MCP_METHODS_CATALOG.to_string();
        if self.mcp_full {
            s.push_str(MCP_METHODS_CATALOG_FULL);
        }
        s
    }

    #[tool(description = "Read full document body and metadata (JSON-RPC get_document).")]
    async fn get_document(&self, Parameters(p): Parameters<PathArg>) -> Result<String, String> {
        self.call_rpc_json("get_document", json!({ "path": p.path }))
            .await
    }

    #[tool(description = "Upsert document body (JSON-RPC put_document).")]
    async fn put_document(&self, Parameters(p): Parameters<PathContent>) -> Result<String, String> {
        self.call_rpc_json(
            "put_document",
            json!({ "path": p.path, "content": p.content }),
        )
        .await
    }

    #[tool(description = "Create a new file (JSON-RPC create_document).")]
    async fn create_document(
        &self,
        Parameters(p): Parameters<PathContent>,
    ) -> Result<String, String> {
        self.call_rpc_json(
            "create_document",
            json!({ "path": p.path, "content": p.content }),
        )
        .await
    }

    #[tool(
        description = "Append bytes to document (JSON-RPC append_document). Not for chat/meeting lines — use say_document so from_id is recorded."
    )]
    async fn append_document(
        &self,
        Parameters(p): Parameters<PathContent>,
    ) -> Result<String, String> {
        self.call_rpc_json(
            "append_document",
            json!({ "path": p.path, "content": p.content }),
        )
        .await
    }

    #[tool(
        description = "Append a markdown chat block; **preferred for meetings and conversations** — `from_id` is the sender nickname recorded in the appended block. Do not include your nickname in `content` (JSON-RPC say_document)."
    )]
    async fn say_document(&self, Parameters(p): Parameters<SayArg>) -> Result<String, String> {
        self.call_rpc_json(
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
        Parameters(p): Parameters<ListDirectoryArg>,
    ) -> Result<String, String> {
        let mut m = Map::new();
        if let Some(ref path) = p.path
            && !path.is_empty()
        {
            m.insert("path".into(), json!(path));
        }
        self.call_rpc_json("list_directory", Value::Object(m)).await
    }

    #[tool(
        description = "Indexed full-text search across document bodies (JSON-RPC search); optional path limits subtree. For path/name in the tree use list_directory; for line-regex in one file use grep."
    )]
    async fn search(&self, Parameters(p): Parameters<SearchArg>) -> Result<String, String> {
        let mut m = Map::new();
        m.insert("query".into(), json!(p.query));
        if let Some(path) = p.path.filter(|s| !s.is_empty()) {
            m.insert("path".into(), json!(path));
        }
        self.call_rpc_json("search", Value::Object(m)).await
    }

    #[tool(
        description = "Create a directory (JSON-RPC create_directory). Optional parents=true creates missing ancestors (mkdir -p)."
    )]
    async fn create_directory(
        &self,
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
        self.call_rpc_json("create_directory", Value::Object(m))
            .await
    }

    #[tool(
        description = "Get or set `description` on a file or directory (JSON-RPC describe). Omit `description` to read; pass a string to set; empty string clears."
    )]
    async fn describe(&self, Parameters(p): Parameters<DescribeArg>) -> Result<String, String> {
        let mut m = Map::new();
        m.insert("path".into(), json!(p.path));
        if let Some(d) = p.description {
            m.insert("description".into(), json!(d));
        }
        self.call_rpc_json("describe", Value::Object(m)).await
    }

    #[tool(description = "Whether path is an existing file (JSON-RPC exists).")]
    async fn document_exists(&self, Parameters(p): Parameters<PathArg>) -> Result<String, String> {
        self.call_rpc_json("exists", json!({ "path": p.path }))
            .await
    }

    #[tool(description = "File metadata and line count (JSON-RPC stat).")]
    async fn stat(&self, Parameters(p): Parameters<PathArg>) -> Result<String, String> {
        self.call_rpc_json("stat", json!({ "path": p.path })).await
    }

    #[tool(description = "Line/word/byte counts (JSON-RPC wc).")]
    async fn wc(&self, Parameters(p): Parameters<PathArg>) -> Result<String, String> {
        self.call_rpc_json("wc", json!({ "path": p.path })).await
    }

    #[tool(
        description = "First N logical lines (JSON-RPC head). `lines` optional (default 10); number or string integer. `0` returns no lines."
    )]
    async fn head(&self, Parameters(p): Parameters<HeadArg>) -> Result<String, String> {
        let mut m = Map::new();
        m.insert("path".into(), json!(p.path));
        if let Some(l) = p.lines {
            m.insert(
                "lines".into(),
                json!(l.parse_u32().map_err(|e| e.to_string())?),
            );
        }
        self.call_rpc_json("head", Value::Object(m)).await
    }

    #[tool(
        description = "Tail of file (JSON-RPC tail). `lines` optional (default 10 last lines); number, string integer (`0` = no lines), or '+N' from-line."
    )]
    async fn tail(&self, Parameters(p): Parameters<TailArg>) -> Result<String, String> {
        let mut m = Map::new();
        m.insert("path".into(), json!(p.path));
        m.insert(
            "lines".into(),
            tail_lines_to_rpc_value(p.lines.as_ref()).map_err(|e| e.to_string())?,
        );
        self.call_rpc_json("tail", Value::Object(m)).await
    }

    #[tool(
        description = "Inclusive 1-based line range (JSON-RPC slice). `start_line`/`end_line` or `from_line`/`to_line`; number or string integer."
    )]
    async fn slice(&self, Parameters(p): Parameters<SliceArg>) -> Result<String, String> {
        let start = p.start_line.parse_u32().map_err(|e| e.to_string())?;
        let end = p.end_line.parse_u32().map_err(|e| e.to_string())?;
        self.call_rpc_json(
            "slice",
            json!({
                "path": p.path,
                "start_line": start,
                "end_line": end,
            }),
        )
        .await
    }

    #[tool(
        description = "Regex line matches within one document (JSON-RPC grep); max_matches 0 = unlimited. Not indexed repo-wide search — use search; not tree listing — use list_directory."
    )]
    async fn grep(&self, Parameters(p): Parameters<GrepArg>) -> Result<String, String> {
        self.call_rpc_json(
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
    async fn wait(&self, Parameters(p): Parameters<PathArg>) -> Result<String, String> {
        self.call_rpc_json("wait", json!({ "path": p.path })).await
    }

    #[tool(
        description = "Delete a file (JSON-RPC delete_document). **Registered only when `mcp.full = true` in server config.**"
    )]
    async fn delete_document(&self, Parameters(p): Parameters<PathArg>) -> Result<String, String> {
        self.call_rpc_json("delete_document", json!({ "path": p.path }))
            .await
    }

    #[tool(
        description = "Delete a directory (JSON-RPC delete_directory). **Registered only when `mcp.full = true`.**"
    )]
    async fn delete_directory(
        &self,
        Parameters(p): Parameters<DeleteDirectoryArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(
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
        Parameters(p): Parameters<RenameDocumentArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(
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
        Parameters(p): Parameters<RenameDirectoryArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(
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
        Parameters(p): Parameters<MoveDocumentArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(
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
        Parameters(p): Parameters<MoveDirectoryArg>,
    ) -> Result<String, String> {
        self.call_rpc_json(
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
        description = "Rebuild search index (JSON-RPC reindex); optional path limits subtree. **Registered only when `mcp.full = true`.**"
    )]
    async fn reindex(&self, Parameters(p): Parameters<ReindexArg>) -> Result<String, String> {
        let mut m = Map::new();
        if let Some(path) = p.path.filter(|s| !s.is_empty()) {
            m.insert("path".into(), json!(path));
        }
        self.call_rpc_json("reindex", Value::Object(m)).await
    }
}

#[tool_handler]
impl ServerHandler for TabulariumMcp {
    fn get_info(&self) -> ServerInfo {
        let instructions = if self.mcp_full {
            "Tabularium MCP — **trusted FULL mode**. Same JSON-RPC semantics and authentication as POST /rpc (no alternate permission model). Includes destructive tools: delete_document, delete_directory, rename_document, rename_directory, move_document, move_directory, reindex — plus the standard librarium surface. **Private / operator-controlled deployments only.**"
        } else {
            "Tabularium MCP — fs-like librarium for machine spirits. Three rites: list_directory (tree walk / find-by-listing, rows carry modified_at), search (indexed full-text across bodies, file names, descriptions), grep (regex lines in one file). say_document for meetings (`from_id` = sender nickname). append_document is not for chat blocks. Destructive tools (delete/move/rename/reindex) are not registered unless `mcp.full = true` in server config; otherwise use CLI or REST."
        };
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(instructions)
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
        "MCP streamable HTTP bound; Omnissiah hears at http://{}/mcp",
        listen
    );
    axum::serve(listener, router).await?;
    Ok(())
}

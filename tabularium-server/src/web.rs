//! Axum REST (`/api/doc/*`, `/api/search`) and JSON-RPC 2.0 (`POST /rpc`).

use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tracing::info;

use crate::auth::{
    RequestAuth, check_read, check_search_directory_traverse, check_write,
    directory_listing_requires_parent_readable, filter_listed_children, require_mgmt_admin,
    resolve_request_auth_arc, whoami_json,
};
use crate::jwt_assertion::AssertionRuntime;
use crate::test_payload::test_payload;
use crate::ws_doc::ws_upgrade;
use tabularium::jsonrpc_codes::{FORBIDDEN, UNAUTHORIZED};
use tabularium::resource_path::{
    canonical_path_segments, join_under_directory, normalize_path_for_rpc,
};
use tabularium::validate_entity_name;
use tabularium::{DocumentWaitStatus, EntryId, EntryKind, Error, SqliteDatabase, TailMode};

const SEARCH_LIMIT: usize = 256;

enum SearchScopeReady {
    Global,
    Directory(String),
    File { path: String, id: EntryId },
}

impl SearchScopeReady {
    fn authorize(&self, auth: &RequestAuth) -> tabularium::Result<()> {
        match self {
            Self::Global => Ok(()),
            Self::Directory(p) => check_search_directory_traverse(auth, p),
            Self::File { path, .. } => check_read(auth, path),
        }
    }

    fn directory_prefix_and_restrict(&self) -> (Option<&str>, Option<EntryId>) {
        match self {
            Self::Global => (None, None),
            Self::Directory(p) => (Some(p.as_str()), None),
            Self::File { id, .. } => (None, Some(*id)),
        }
    }
}

async fn resolve_search_scope_normalized(
    db: &SqliteDatabase,
    normalized: String,
) -> tabularium::Result<SearchScopeReady> {
    let trimmed = normalized.trim_end_matches('/').to_string();
    if trimmed.is_empty() {
        return Ok(SearchScopeReady::Global);
    }
    match db.resolve_file_path(&trimmed).await {
        Ok(id) => Ok(SearchScopeReady::File { path: trimmed, id }),
        Err(Error::NotFound(_)) => {
            db.resolve_directory_path(&trimmed).await?;
            Ok(SearchScopeReady::Directory(trimmed))
        }
        Err(e) => Err(e),
    }
}

/// Random pre-shared key: `A–Z`, `a–z`, `0–9` (`44` chars, ~260 bits).
fn generate_access_psk() -> String {
    use rand::Rng;
    use rand::rngs::OsRng;

    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = OsRng;
    (0..44)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<SqliteDatabase>,
    pub wait_timeout: Duration,
    pub process_started_at: bma_ts::Monotonic,
    /// REST + main HTTP JSON-RPC (`POST /rpc`).
    pub authenticate_api: bool,
    /// MCP streamable HTTP (same `AppState`).
    pub authenticate_mcp: bool,
    /// Optional `[oidc]` upstream JWT assertion verifier (JWKS).
    pub oidc: Option<Arc<AssertionRuntime>>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/test", get(api_test))
        .route("/api/whoami", get(api_whoami))
        .route("/api/doc", get(list_root).post(rest_mkdir))
        .route(
            "/api/doc/{*rest}",
            get(get_or_list)
                .post(rest_post_legacy_document)
                .put(rest_put_document)
                .patch(rest_patch_document)
                .delete(rest_delete_any),
        )
        .route("/api/search", get(search_get).post(search_post))
        .route("/ws", get(ws_upgrade))
        .route("/rpc", post(rpc_dispatch))
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(state)
        .fallback(axum::routing::get(crate::embedded_ui::serve))
}

async fn api_test(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    Ok(Json(test_payload(
        st.process_started_at,
        st.authenticate_api,
        st.oidc.is_some(),
    )))
}

async fn api_whoami(State(st): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let auth = api_resolve_auth(&st, &headers).await?;
    Ok(Json(whoami_json(&auth)))
}

async fn api_resolve_auth(st: &AppState, headers: &HeaderMap) -> Result<RequestAuth, ApiError> {
    Ok(if !st.authenticate_api {
        RequestAuth::Disabled
    } else {
        resolve_request_auth_arc(st.db.as_ref(), true, headers, st.oidc.as_ref())
            .await
            .map_err(ApiError)?
    })
}

fn rest_to_canonical(rest: &str) -> String {
    let t = rest.trim().trim_start_matches('/');
    if t.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", t)
    }
}

#[derive(Debug, Deserialize)]
struct CreateDirectoryBody {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    name: Option<String>,
    description: Option<String>,
    /// When true, create missing parent directories (POSIX `mkdir -p`); best-effort / non-atomic.
    #[serde(default)]
    parents: bool,
}

fn resolve_create_directory_body(body: &CreateDirectoryBody) -> tabularium::Result<String> {
    match (&body.path, &body.name) {
        (Some(p), _) => {
            let t = p.trim();
            if t.contains("//") {
                return Err(Error::InvalidInput(
                    "path must not contain empty segment (//)".into(),
                ));
            }
            normalize_path_for_rpc(t)
        }
        (_, Some(n)) => {
            let n = n.trim();
            if n.is_empty() {
                return Err(Error::InvalidInput("name must not be empty".into()));
            }
            if n.contains('/') || n.contains('\\') {
                return Err(Error::InvalidInput("name must not contain '/'".into()));
            }
            validate_entity_name(n)?;
            Ok(format!("/{n}"))
        }
        (None, None) => Err(Error::InvalidInput("missing path or name".into())),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyCreateDocumentBody {
    name: String,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PutDocumentBody {
    content: String,
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default)]
    dir: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchPostBody {
    q: String,
    #[serde(default)]
    dir: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DocGetQuery {
    #[serde(default)]
    wait: bool,
}

#[derive(Debug, Deserialize)]
struct DirDeleteQuery {
    #[serde(default)]
    recursive: bool,
}

struct ApiError(Error);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg): (StatusCode, String) = match &self.0 {
            Error::Unauthorized(s) => (StatusCode::UNAUTHORIZED, s.clone()),
            Error::Forbidden(s) => (StatusCode::FORBIDDEN, s.clone()),
            Error::NotFound(s) => (StatusCode::NOT_FOUND, s.clone()),
            Error::Duplicate(s) | Error::NotEmpty(s) | Error::RevisionMismatch(s) => {
                (StatusCode::CONFLICT, s.clone())
            }
            Error::InvalidInput(s) => (StatusCode::BAD_REQUEST, s.clone()),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.0.to_string()),
        };
        let body = json!({ "error": msg });
        (status, Json(body)).into_response()
    }
}

impl<E: Into<Error>> From<E> for ApiError {
    fn from(value: E) -> Self {
        ApiError(value.into())
    }
}

type ApiResult<T> = std::result::Result<T, ApiError>;

/// JSON-RPC dispatch errors — mapped to JSON-RPC 2.0 codes in [`rpc_dispatch`].
pub(crate) enum RpcAppError {
    MethodNotFound(String),
    Other(Error),
}

impl From<Error> for RpcAppError {
    fn from(e: Error) -> Self {
        RpcAppError::Other(e)
    }
}

fn is_multipart_form(ct: &str) -> bool {
    ct.to_ascii_lowercase().starts_with("multipart/form-data")
}

fn is_www_form_urlencoded(ct: &str) -> bool {
    ct.to_ascii_lowercase()
        .starts_with("application/x-www-form-urlencoded")
}

/// JSON `{ "content": "..." }`, `application/x-www-form-urlencoded`, raw UTF-8 body, or multipart field `content`.
async fn extract_put_patch_content(
    ct: &str,
    body: Bytes,
    allow_empty_raw_body: bool,
) -> Result<String, ApiError> {
    if is_multipart_form(ct) {
        let boundary =
            multer::parse_boundary(ct).map_err(|e| ApiError(Error::InvalidInput(e.to_string())))?;
        let mut fields = crate::multipart_body::form_fields(body, boundary)
            .await
            .map_err(ApiError)?;
        return fields.remove("content").ok_or_else(|| {
            ApiError(Error::InvalidInput(
                "multipart field content required".into(),
            ))
        });
    }
    if is_www_form_urlencoded(ct) {
        let b: PutDocumentBody = serde_urlencoded::from_bytes(&body)
            .map_err(|e| ApiError(Error::InvalidInput(e.to_string())))?;
        return Ok(b.content);
    }
    if body.is_empty() {
        if allow_empty_raw_body {
            return Ok(String::new());
        }
        return Err(ApiError(Error::InvalidInput("empty body".into())));
    }
    match serde_json::from_slice::<PutDocumentBody>(&body) {
        Ok(b) => Ok(b.content),
        Err(_) => Ok(String::from_utf8_lossy(&body).into_owned()),
    }
}

fn normalize_put_content(content: String) -> String {
    if content.trim_end_matches(['\r', '\n']).is_empty() {
        String::new()
    } else {
        content
    }
}

async fn list_root(State(st): State<AppState>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let auth = api_resolve_auth(&st, &headers).await?;
    list_directory_inner(&st, "/", &auth).await
}

async fn list_directory_inner(
    st: &AppState,
    dir_path: &str,
    auth: &RequestAuth,
) -> ApiResult<Json<Value>> {
    if directory_listing_requires_parent_readable(auth, dir_path) {
        check_read(auth, dir_path).map_err(ApiError)?;
    }
    let rows = st.db.list_directory(dir_path).await?;
    let rows = filter_listed_children(auth, dir_path, rows);
    let v: Vec<Value> = rows
        .into_iter()
        .map(|e| {
            json!({
                "id": e.id().raw(),
                "kind": e.kind() as i64,
                "name": e.name(),
                "description": e.description(),
                "created_at": e.created_at(),
                "modified_at": e.modified_at(),
                "accessed_at": e.accessed_at(),
                "revision": e.revision(),
                "size_bytes": e.size_bytes(),
                "recursive_file_count": e.recursive_file_count(),
            })
        })
        .collect();
    Ok(Json(Value::Array(v)))
}

async fn rest_mkdir(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateDirectoryBody>,
) -> ApiResult<(StatusCode, HeaderMap, Json<Value>)> {
    let auth = api_resolve_auth(&st, &headers).await?;
    let path = resolve_create_directory_body(&body).map_err(ApiError)?;
    canonical_path_segments(&path).map_err(ApiError)?;
    check_write(&auth, &path).map_err(ApiError)?;
    let id = st
        .db
        .create_directory(&path, body.description.as_deref(), body.parents)
        .await?;
    let loc = format!("/api/doc{}", path);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::LOCATION,
        HeaderValue::from_str(&loc).map_err(|e| ApiError(Error::InvalidInput(e.to_string())))?,
    );
    Ok((
        StatusCode::CREATED,
        headers,
        Json(json!({"id": id.raw(), "path": path})),
    ))
}

async fn get_or_list(
    State(st): State<AppState>,
    Path(rest): Path<String>,
    Query(query): Query<DocGetQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let auth = api_resolve_auth(&st, &headers).await?;
    let canon = rest_to_canonical(&rest);
    match st.db.resolve_existing_file_path(&canon).await {
        Ok(fid) => {
            check_read(&auth, &canon).map_err(ApiError)?;
            if query.wait {
                return match st
                    .db
                    .wait_until_document_changed(fid, st.wait_timeout)
                    .await?
                {
                    DocumentWaitStatus::Changed => Ok(StatusCode::NO_CONTENT.into_response()),
                    DocumentWaitStatus::TimedOut => Ok(StatusCode::GATEWAY_TIMEOUT.into_response()),
                };
            }
            let (meta, body) = st.db.cat_document_bundle(fid).await?;
            Ok(Json(json!({
                "id": meta.id().raw(),
                "path": meta.canonical_path(),
                "content": body,
                "created_at": meta.created_at(),
                "modified_at": meta.modified_at(),
                "accessed_at": meta.accessed_at(),
                "revision": meta.revision(),
                "size_bytes": meta.size_bytes(),
            }))
            .into_response())
        }
        Err(Error::NotFound(_)) => {
            let j = list_directory_inner(&st, &canon, &auth).await?;
            Ok(j.into_response())
        }
        Err(e) => Err(ApiError(e)),
    }
}

/// Legacy `POST /api/doc/{parent}` with `{"name","content"}` (pre–absolute-path wire shape).
async fn rest_post_legacy_document(
    State(st): State<AppState>,
    Path(rest): Path<String>,
    headers: HeaderMap,
    Json(body): Json<LegacyCreateDocumentBody>,
) -> ApiResult<(StatusCode, HeaderMap)> {
    let auth = api_resolve_auth(&st, &headers).await?;
    let parent = rest_to_canonical(&rest);
    canonical_path_segments(&parent).map_err(ApiError)?;
    validate_entity_name(&body.name).map_err(ApiError)?;
    let doc_path = if parent == "/" {
        format!("/{}", body.name)
    } else {
        format!("{}/{}", parent, body.name)
    };
    canonical_path_segments(&doc_path).map_err(ApiError)?;
    check_write(&auth, &doc_path).map_err(ApiError)?;
    let id = st
        .db
        .create_document_at_path(&doc_path, &body.content, false, None)
        .await
        .map_err(ApiError)?;
    let loc = format!("/api/doc/{}", doc_path.trim_start_matches('/'));
    let mut headers = HeaderMap::new();
    headers.insert(
        header::LOCATION,
        HeaderValue::from_str(&loc).map_err(|e| ApiError(Error::InvalidInput(e.to_string())))?,
    );
    info!(
        target: "tabularium_server::api",
        path = %doc_path,
        document_id = id.raw(),
        "REST legacy create document"
    );
    Ok((StatusCode::CREATED, headers))
}

async fn rest_put_document(
    State(st): State<AppState>,
    Path(rest): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let auth = api_resolve_auth(&st, &headers).await?;
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let content = normalize_put_content(extract_put_patch_content(ct, body, true).await?);
    let canon = rest_to_canonical(&rest);
    check_write(&auth, &canon).map_err(ApiError)?;
    st.db
        .put_document_by_path(&canon, &content, true, None)
        .await?;
    info!(
        target: "tabularium_server::api",
        path = %canon,
        "REST replace document"
    );
    Ok(StatusCode::NO_CONTENT)
}

async fn rest_patch_document(
    State(st): State<AppState>,
    Path(rest): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let auth = api_resolve_auth(&st, &headers).await?;
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let content = extract_put_patch_content(ct, body, false).await?;
    let canon = rest_to_canonical(&rest);
    check_write(&auth, &canon).map_err(ApiError)?;
    st.db
        .append_document_by_path(&canon, &content, true)
        .await?;
    info!(
        target: "tabularium_server::api",
        path = %canon,
        append_len = content.len(),
        "REST append document"
    );
    Ok(StatusCode::NO_CONTENT)
}

async fn rest_delete_any(
    State(st): State<AppState>,
    Path(rest): Path<String>,
    Query(q): Query<DirDeleteQuery>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    let auth = api_resolve_auth(&st, &headers).await?;
    let canon = rest_to_canonical(&rest);
    check_write(&auth, &canon).map_err(ApiError)?;
    match st.db.resolve_existing_file_path(&canon).await {
        Ok(fid) => {
            st.db.delete_document(fid).await?;
            info!(
                target: "tabularium_server::api",
                path = %canon,
                document_id = fid.raw(),
                "REST delete document"
            );
        }
        Err(Error::NotFound(_)) => {
            if q.recursive {
                st.db.delete_directory_recursive(&canon).await?;
            } else {
                st.db.delete_directory(&canon).await?;
            }
        }
        Err(e) => return Err(ApiError(e)),
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn search_get(
    State(st): State<AppState>,
    Query(q): Query<SearchQuery>,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    let auth = api_resolve_auth(&st, &headers).await?;
    let dir = q.dir.as_deref();
    search_inner(&st, &auth, &q.q, dir).await
}

async fn search_post(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    let auth = api_resolve_auth(&st, &headers).await?;
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let (q, dir_owned): (String, Option<String>) = if is_multipart_form(ct) {
        let boundary =
            multer::parse_boundary(ct).map_err(|e| ApiError(Error::InvalidInput(e.to_string())))?;
        let mut fields = crate::multipart_body::form_fields(body, boundary)
            .await
            .map_err(ApiError)?;
        let q = fields
            .remove("q")
            .ok_or_else(|| ApiError(Error::InvalidInput("multipart field q required".into())))?;
        let dir = fields.remove("dir");
        (q, dir)
    } else if is_www_form_urlencoded(ct) {
        let b: SearchPostBody = serde_urlencoded::from_bytes(&body)
            .map_err(|e| ApiError(Error::InvalidInput(e.to_string())))?;
        (b.q, b.dir)
    } else {
        let b: SearchPostBody = serde_json::from_slice(&body)
            .map_err(|e| ApiError(Error::InvalidInput(e.to_string())))?;
        (b.q, b.dir)
    };
    let dir = dir_owned.as_deref();
    search_inner(&st, &auth, &q, dir).await
}

fn filter_search_hits(
    auth: &RequestAuth,
    hits: Vec<tabularium::SearchHit>,
) -> Vec<tabularium::SearchHit> {
    match auth {
        RequestAuth::Disabled => hits,
        RequestAuth::Authenticated(ctx) if ctx.admin() => hits,
        RequestAuth::Authenticated(ctx) => hits
            .into_iter()
            .filter(|h| ctx.check_read_abs(h.path()).is_ok())
            .collect(),
    }
}

async fn search_inner(
    st: &AppState,
    auth: &RequestAuth,
    q: &str,
    dir_prefix: Option<&str>,
) -> ApiResult<Json<Value>> {
    let scope = if let Some(d) = dir_prefix {
        let n = normalize_path_for_rpc(d).map_err(ApiError)?;
        canonical_path_segments(&n).map_err(ApiError)?;
        resolve_search_scope_normalized(&st.db, n)
            .await
            .map_err(ApiError)?
    } else {
        SearchScopeReady::Global
    };
    scope.authorize(auth).map_err(ApiError)?;
    let (dp, restrict_doc) = scope.directory_prefix_and_restrict();
    let hits = st.db.search_hits(q, dp, SEARCH_LIMIT, restrict_doc).await?;
    let hits = filter_search_hits(auth, hits);
    let v: Vec<Value> = hits
        .into_iter()
        .map(|h| {
            json!({
                "document_id": h.document_id().raw(),
                "path": h.path(),
                "snippet": h.snippet(),
                "score": h.score(),
                "line_number": h.line_number(),
            })
        })
        .collect();
    Ok(Json(Value::Array(v)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonRpcRequest {
    jsonrpc: String,
    method: String,
    #[serde(default)]
    params: Option<Value>,
    id: Option<Value>,
}

async fn rpc_dispatch(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Json<Value> {
    let parsed: serde_json::Result<JsonRpcRequest> = serde_json::from_slice(&body);
    let req = match parsed {
        Ok(r) => r,
        Err(e) => {
            return Json(rpc_error(None, -32700, &e.to_string()));
        }
    };
    if req.jsonrpc != "2.0" {
        return Json(rpc_error(req.id, -32600, "invalid jsonrpc version"));
    }
    info!(
        target: "tabularium_server::rpc",
        transport = "http",
        method = %req.method,
        params = %crate::rpc_preview::format_rpc_params_preview(req.params.as_ref()),
        "RPC request"
    );
    let auth_result: Result<RequestAuth, Error> = if !st.authenticate_api {
        Ok(RequestAuth::Disabled)
    } else if req.method == "test" {
        // Uptime rite stays callable so the machine-spirits can probe the Throne without a key.
        Ok(RequestAuth::Disabled)
    } else {
        resolve_request_auth_arc(st.db.as_ref(), true, &headers, st.oidc.as_ref()).await
    };
    let auth = match auth_result {
        Ok(a) => a,
        Err(e) => {
            return Json(match &e {
                Error::Unauthorized(s) => rpc_error(req.id, UNAUTHORIZED, s),
                _ => rpc_error(req.id, -32603, &e.to_string()),
            });
        }
    };
    let map = params_map(req.params.as_ref());
    let rpc_result = dispatch_app_rpc(&st, &auth, &req.method, map).await;
    match rpc_result {
        Ok(v) => Json(json!({
            "jsonrpc": "2.0",
            "result": v,
            "id": req.id,
        })),
        Err(RpcAppError::MethodNotFound(msg)) => Json(rpc_error(req.id, -32601, &msg)),
        Err(RpcAppError::Other(e)) => match &e {
            Error::Unauthorized(s) => Json(rpc_error(req.id, UNAUTHORIZED, s)),
            Error::Forbidden(s) => Json(rpc_error(req.id, FORBIDDEN, s)),
            Error::InvalidInput(s) | Error::NotEmpty(s) => Json(rpc_error(req.id, -32602, s)),
            Error::Duplicate(_) => Json(rpc_error(
                req.id,
                tabularium::jsonrpc_codes::DUPLICATE_RESOURCE,
                &e.to_string(),
            )),
            Error::RevisionMismatch(_) => Json(rpc_error(
                req.id,
                tabularium::jsonrpc_codes::REVISION_MISMATCH,
                &e.to_string(),
            )),
            _ => Json(rpc_error(req.id, -32603, &e.to_string())),
        },
    }
}

fn rpc_error(id: Option<Value>, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "error": { "code": code, "message": message },
        "id": id,
    })
}

fn params_map(params: Option<&Value>) -> Map<String, Value> {
    match params {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    }
}

fn get_str<'a>(m: &'a Map<String, Value>, key: &str) -> tabularium::Result<&'a str> {
    m.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidInput(format!("missing string param {key}")))
}

/// Optional boolean RPC param. Missing / `null` defaults to `false`.
/// Accepts JSON `true` / `false`, `"true"` / `"false"` / `"1"` / `"0"` (case-insensitive).
fn get_optional_uuid_str(m: &Map<String, Value>, key: &str) -> tabularium::Result<Option<String>> {
    match m.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                return Err(Error::InvalidInput(format!("param {key}: empty uuid")));
            }
            uuid::Uuid::parse_str(t)
                .map_err(|_| Error::InvalidInput(format!("param {key}: invalid uuid")))?;
            Ok(Some(t.to_string()))
        }
        _ => Err(Error::InvalidInput(format!(
            "param {key}: expected uuid string or null"
        ))),
    }
}

fn get_optional_bool(m: &Map<String, Value>, key: &str) -> tabularium::Result<bool> {
    match m.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Ok(true),
            "false" | "0" | "no" | "" => Ok(false),
            other => Err(Error::InvalidInput(format!(
                "param {key}: expected boolean, got {other:?}"
            ))),
        },
        _ => Err(Error::InvalidInput(format!(
            "param {key}: expected boolean"
        ))),
    }
}

fn json_u32_loose(v: &Value, key: &str) -> tabularium::Result<u32> {
    match v {
        Value::Number(n) => {
            let u = n
                .as_u64()
                .ok_or_else(|| Error::InvalidInput(format!("param {key}: number out of range")))?;
            u32::try_from(u).map_err(|_| Error::InvalidInput(format!("param {key} out of range")))
        }
        Value::String(s) => s.trim().parse::<u32>().map_err(|_| {
            Error::InvalidInput(format!("param {key}: expected non-negative integer"))
        }),
        _ => Err(Error::InvalidInput(format!(
            "param {key}: expected number or string integer"
        ))),
    }
}

/// GNU `head` default: 10 lines when `lines` is omitted.
fn get_head_lines(m: &Map<String, Value>) -> tabularium::Result<u32> {
    match m.get("lines") {
        None | Some(Value::Null) => Ok(10),
        Some(v) => json_u32_loose(v, "lines"),
    }
}

/// GNU `tail` default: last 10 lines when `lines` is omitted.
fn get_tail_mode_optional(m: &Map<String, Value>, key: &str) -> tabularium::Result<TailMode> {
    match m.get(key) {
        None | Some(Value::Null) => Ok(TailMode::Last(10)),
        Some(v) => match v {
            Value::Number(n) => {
                let u = n.as_u64().ok_or_else(|| {
                    Error::InvalidInput(format!("param {key}: number out of range"))
                })?;
                u32::try_from(u)
                    .map(TailMode::Last)
                    .map_err(|_| Error::InvalidInput(format!("param {key} out of range")))
            }
            Value::String(s) => TailMode::from_plus_wire_str(s).map_err(Error::InvalidInput),
            _ => Err(Error::InvalidInput(format!(
                "param {key}: expected number or \"+N\" string"
            ))),
        },
    }
}

fn get_slice_line(m: &Map<String, Value>, primary: &str, alias: &str) -> tabularium::Result<u32> {
    if let Some(v) = m.get(primary).filter(|v| !v.is_null()) {
        return json_u32_loose(v, primary);
    }
    if let Some(v) = m.get(alias).filter(|v| !v.is_null()) {
        return json_u32_loose(v, alias);
    }
    Err(Error::InvalidInput(format!(
        "missing param {primary} (or {alias})"
    )))
}

fn rpc_path(m: &Map<String, Value>, key: &str) -> tabularium::Result<String> {
    normalize_path_for_rpc(get_str(m, key)?)
}

fn rpc_new_dir_path(m: &Map<String, Value>) -> tabularium::Result<String> {
    let s = get_str(m, "path")?;
    normalize_path_for_rpc(s)
}

fn list_directory_rpc_path(m: &Map<String, Value>) -> tabularium::Result<String> {
    match m.get("path").and_then(|v| v.as_str()) {
        None | Some("") => Ok("/".to_string()),
        Some(s) => normalize_path_for_rpc(s),
    }
}

fn directory_search_prefix(m: &Map<String, Value>) -> tabularium::Result<Option<String>> {
    match m.get("path").and_then(|v| v.as_str()) {
        None | Some("" | "/") => Ok(None),
        Some(s) => {
            let n = normalize_path_for_rpc(s)?;
            canonical_path_segments(&n)?;
            Ok(Some(n.trim_end_matches('/').to_string()))
        }
    }
}

/// JSON-RPC dispatch for HTTP `/rpc` and the MCP tool layer (transport glue).
#[allow(clippy::too_many_lines)]
pub(crate) async fn dispatch_app_rpc(
    st: &AppState,
    auth: &RequestAuth,
    method: &str,
    m: Map<String, Value>,
) -> Result<Value, RpcAppError> {
    match method {
        "list_directory" => {
            let p = list_directory_rpc_path(&m)?;
            canonical_path_segments(&p)?;
            if directory_listing_requires_parent_readable(auth, &p) {
                check_read(auth, &p).map_err(RpcAppError::Other)?;
            }
            let rows = st.db.list_directory(&p).await?;
            let rows = filter_listed_children(auth, &p, rows);
            let v: Vec<Value> = rows
                .into_iter()
                .map(|e| {
                    json!({
                        "id": e.id().raw(),
                        "kind": e.kind() as i64,
                        "name": e.name(),
                        "description": e.description(),
                        "created_at": e.created_at(),
                        "modified_at": e.modified_at(),
                        "accessed_at": e.accessed_at(),
                        "revision": e.revision(),
                        "size_bytes": e.size_bytes(),
                        "recursive_file_count": e.recursive_file_count(),
                    })
                })
                .collect();
            Ok(Value::Array(v))
        }
        "create_directory" => {
            let path = rpc_new_dir_path(&m)?;
            canonical_path_segments(&path)?;
            check_write(auth, &path).map_err(RpcAppError::Other)?;
            let description = m
                .get("description")
                .and_then(|v| v.as_str().map(ToString::to_string));
            let parents = m.get("parents").and_then(Value::as_bool).unwrap_or(false);
            let id = st
                .db
                .create_directory(&path, description.as_deref(), parents)
                .await?;
            Ok(json!(id.raw()))
        }
        "delete_directory" => {
            let p = rpc_path(&m, "path")?;
            canonical_path_segments(&p)?;
            check_write(auth, &p).map_err(RpcAppError::Other)?;
            let recursive = m.get("recursive").and_then(Value::as_bool).unwrap_or(false);
            if recursive {
                st.db.delete_directory_recursive(&p).await?;
            } else {
                st.db.delete_directory(&p).await?;
            }
            Ok(Value::Null)
        }
        "rename_directory" => {
            let old = rpc_path(&m, "path")?;
            let newp = rpc_path(&m, "new_path")?;
            canonical_path_segments(&old)?;
            canonical_path_segments(&newp)?;
            check_write(auth, &old).map_err(RpcAppError::Other)?;
            check_write(auth, &newp).map_err(RpcAppError::Other)?;
            st.db.rename_directory(&old, &newp).await?;
            Ok(Value::Null)
        }
        "move_directory" => {
            let src = rpc_path(&m, "path")?;
            let parent = rpc_path(&m, "new_parent")?;
            let new_name = get_str(&m, "new_name")?;
            canonical_path_segments(&src)?;
            canonical_path_segments(&parent)?;
            check_write(auth, &src).map_err(RpcAppError::Other)?;
            let dst = join_under_directory(&parent, new_name);
            check_write(auth, &dst).map_err(RpcAppError::Other)?;
            st.db.move_directory(&src, &parent, new_name).await?;
            Ok(Value::Null)
        }
        "list_documents" => {
            let p = rpc_path(&m, "path")?;
            canonical_path_segments(&p)?;
            if directory_listing_requires_parent_readable(auth, &p) {
                check_read(auth, &p).map_err(RpcAppError::Other)?;
            }
            let rows = st.db.list_directory(&p).await?;
            let rows = filter_listed_children(auth, &p, rows);
            let v: Vec<Value> = rows
                .into_iter()
                .filter(|e| e.kind() == EntryKind::File)
                .map(|d| {
                    json!({
                        "id": d.id().raw(),
                        "name": d.name(),
                        "created_at": d.created_at(),
                        "modified_at": d.modified_at(),
                        "accessed_at": d.accessed_at(),
                        "revision": d.revision(),
                        "size_bytes": d.size_bytes().unwrap_or(0),
                    })
                })
                .collect();
            Ok(Value::Array(v))
        }
        "create_document" => {
            let path = rpc_path(&m, "path")?;
            let content = get_str(&m, "content")?;
            canonical_path_segments(&path)?;
            check_write(auth, &path).map_err(RpcAppError::Other)?;
            let force = get_optional_bool(&m, "force")?;
            let only = get_optional_uuid_str(&m, "only_if_revision")?;
            let id = st
                .db
                .create_document_at_path(&path, content, force, only.as_deref())
                .await?;
            let meta = st.db.document_ref_by_path(&path).await?;
            info!(
                target: "tabularium_server::api",
                method = "create_document",
                path = %path,
                document_id = id.raw(),
                "RPC document write"
            );
            Ok(json!({
                "id": id.raw(),
                "revision": meta.revision(),
            }))
        }
        "put_document" => {
            let path = rpc_path(&m, "path")?;
            let content = normalize_put_content(get_str(&m, "content")?.to_owned());
            let force = get_optional_bool(&m, "force")?;
            let only = get_optional_uuid_str(&m, "only_if_revision")?;
            canonical_path_segments(&path)?;
            check_write(auth, &path).map_err(RpcAppError::Other)?;
            st.db
                .put_document_by_path(&path, &content, force, only.as_deref())
                .await?;
            let meta = st.db.document_ref_by_path(&path).await?;
            info!(
                target: "tabularium_server::api",
                method = "put_document",
                path = %path,
                op = "put",
                force,
                "RPC document write"
            );
            Ok(json!({ "revision": meta.revision() }))
        }
        "delete_document" => {
            let path = rpc_path(&m, "path")?;
            check_write(auth, &path).map_err(RpcAppError::Other)?;
            let fid = st.db.resolve_existing_file_path(&path).await?;
            st.db.delete_document(fid).await?;
            info!(
                target: "tabularium_server::api",
                method = "delete_document",
                path = %path,
                document_id = fid.raw(),
                "RPC document write"
            );
            Ok(Value::Null)
        }
        "update_document" | "replace_document" => {
            let path = rpc_path(&m, "path")?;
            let content = get_str(&m, "content")?;
            check_write(auth, &path).map_err(RpcAppError::Other)?;
            let fid = st.db.resolve_existing_file_path(&path).await?;
            st.db.update_document(fid, content).await?;
            let meta = st.db.get_document_meta(fid).await?;
            info!(
                target: "tabularium_server::api",
                method = method,
                path = %path,
                document_id = fid.raw(),
                "RPC document write"
            );
            Ok(json!({ "revision": meta.revision() }))
        }
        "append_document" => {
            let path = rpc_path(&m, "path")?;
            let content = get_str(&m, "content")?;
            let force = get_optional_bool(&m, "force")?;
            check_write(auth, &path).map_err(RpcAppError::Other)?;
            st.db.append_document_by_path(&path, content, force).await?;
            let meta = st.db.document_ref_by_path(&path).await?;
            info!(
                target: "tabularium_server::api",
                method = "append_document",
                path = %path,
                append_len = content.len(),
                force,
                "RPC document write"
            );
            Ok(json!({ "revision": meta.revision() }))
        }
        "append_if_not_contains" => {
            let path = rpc_path(&m, "path")?;
            let marker = get_str(&m, "marker")?;
            let content = get_str(&m, "content")?;
            canonical_path_segments(&path)?;
            check_write(auth, &path).map_err(RpcAppError::Other)?;
            let modified = st
                .db
                .append_if_not_contains_by_path(&path, marker, content)
                .await?;
            let meta = st.db.document_ref_by_path(&path).await?;
            info!(
                target: "tabularium_server::api",
                method = "append_if_not_contains",
                path = %path,
                marker_len = marker.len(),
                append_len = content.len(),
                modified,
                "RPC conditional append"
            );
            Ok(json!({
                "appended": modified,
                "revision": meta.revision(),
            }))
        }
        "say_document" => {
            let path = rpc_path(&m, "path")?;
            let from_id = get_str(&m, "from_id")?;
            let content = get_str(&m, "content")?;
            check_write(auth, &path).map_err(RpcAppError::Other)?;
            st.db.say_document_by_path(&path, from_id, content).await?;
            let meta = st.db.document_ref_by_path(&path).await?;
            info!(
                target: "tabularium_server::api",
                method = "say_document",
                path = %path,
                from_id = %from_id,
                "RPC document write"
            );
            Ok(json!({ "revision": meta.revision() }))
        }
        "touch_document" => {
            let path = rpc_path(&m, "path")?;
            canonical_path_segments(&path)?;
            check_write(auth, &path).map_err(RpcAppError::Other)?;
            let modified_at = match m.get("modified_at") {
                None | Some(Value::Null) => None,
                Some(v) => Some(serde_json::from_value(v.clone()).map_err(|e| {
                    Error::InvalidInput(format!("touch_document: invalid modified_at: {e}"))
                })?),
            };
            st.db.touch_document_by_path(&path, modified_at).await?;
            let meta = match st.db.document_ref_by_path(&path).await {
                Ok(m) => m,
                Err(_) => {
                    return Ok(json!({ "revision": Value::Null }));
                }
            };
            info!(
                target: "tabularium_server::api",
                method = "touch_document",
                path = %path,
                "RPC document write"
            );
            Ok(json!({ "revision": meta.revision() }))
        }
        "rename_document" => {
            let path = rpc_path(&m, "path")?;
            let new_name = get_str(&m, "new_name")?;
            check_write(auth, &path).map_err(RpcAppError::Other)?;
            let fid = st.db.resolve_existing_file_path(&path).await?;
            let (parent, _) = tabularium::resource_path::parent_and_final_name(&path)
                .map_err(RpcAppError::Other)?;
            let new_path = join_under_directory(&parent, new_name);
            check_write(auth, &new_path).map_err(RpcAppError::Other)?;
            st.db.rename_document(fid, new_name).await?;
            let meta = st.db.get_document_meta(fid).await?;
            Ok(json!({ "revision": meta.revision() }))
        }
        "move_document" => {
            let path = rpc_path(&m, "path")?;
            let new_path = rpc_path(&m, "new_path")?;
            canonical_path_segments(&path)?;
            canonical_path_segments(&new_path)?;
            check_write(auth, &path).map_err(RpcAppError::Other)?;
            check_write(auth, &new_path).map_err(RpcAppError::Other)?;
            let fid = st.db.resolve_existing_file_path(&path).await?;
            let (parent, name) = tabularium::resource_path::parent_and_final_name(&new_path)
                .map_err(RpcAppError::Other)?;
            st.db.move_document_to_directory(fid, &parent, name).await?;
            let meta = st.db.get_document_meta(fid).await?;
            Ok(json!({ "revision": meta.revision() }))
        }
        "copy_entries" => {
            let src = rpc_path(&m, "src")?;
            let dst = rpc_path(&m, "dst")?;
            let recursive = m.get("recursive").and_then(Value::as_bool).unwrap_or(false);
            canonical_path_segments(&src)?;
            canonical_path_segments(&dst)?;
            check_read(auth, &src).map_err(RpcAppError::Other)?;
            check_write(auth, &dst).map_err(RpcAppError::Other)?;
            st.db.cp(&src, &dst, recursive).await?;
            Ok(Value::Null)
        }
        "get_document" | "cat" => {
            let path = rpc_path(&m, "path")?;
            check_read(auth, &path).map_err(RpcAppError::Other)?;
            let fid = st.db.resolve_existing_file_path(&path).await?;
            let (meta, body) = st.db.cat_document_bundle(fid).await?;
            Ok(json!({
                "id": meta.id().raw(),
                "path": meta.canonical_path(),
                "content": body,
                "created_at": meta.created_at(),
                "modified_at": meta.modified_at(),
                "accessed_at": meta.accessed_at(),
                "revision": meta.revision(),
                "size_bytes": meta.size_bytes(),
            }))
        }
        "wait" => {
            let path = rpc_path(&m, "path")?;
            check_read(auth, &path).map_err(RpcAppError::Other)?;
            let fid = st.db.resolve_existing_file_path(&path).await?;
            match st
                .db
                .wait_until_document_changed(fid, st.wait_timeout)
                .await?
            {
                DocumentWaitStatus::Changed => Ok(Value::Null),
                DocumentWaitStatus::TimedOut => Err(RpcAppError::Other(Error::InvalidInput(
                    "wait timed out".into(),
                ))),
            }
        }
        "get_document_ref" => {
            let path = rpc_path(&m, "path")?;
            check_read(auth, &path).map_err(RpcAppError::Other)?;
            let meta = st.db.document_ref_by_path(&path).await?;
            Ok(json!({
                "id": meta.id().raw(),
                "path": meta.canonical_path(),
                "name": meta.name(),
                "created_at": meta.created_at(),
                "modified_at": meta.modified_at(),
                "accessed_at": meta.accessed_at(),
                "revision": meta.revision(),
                "size_bytes": meta.size_bytes(),
            }))
        }
        "exists" => {
            let path = rpc_path(&m, "path")?;
            let ex = st.db.document_exists_at_path(&path).await?;
            if ex {
                check_read(auth, &path).map_err(RpcAppError::Other)?;
            } else if directory_listing_requires_parent_readable(auth, &path) {
                check_read(auth, &path).map_err(RpcAppError::Other)?;
            }
            Ok(json!(ex))
        }
        "search" => {
            let q = get_str(&m, "query")?;
            let prefix = directory_search_prefix(&m)?;
            let scope = match prefix {
                None => SearchScopeReady::Global,
                Some(p) => resolve_search_scope_normalized(&st.db, p)
                    .await
                    .map_err(RpcAppError::Other)?,
            };
            scope.authorize(auth).map_err(RpcAppError::Other)?;
            let (dp, restrict_doc) = scope.directory_prefix_and_restrict();
            let hits = st.db.search_hits(q, dp, SEARCH_LIMIT, restrict_doc).await?;
            let hits = filter_search_hits(auth, hits);
            let v: Vec<Value> = hits
                .into_iter()
                .map(|h| {
                    json!({
                        "document_id": h.document_id().raw(),
                        "path": h.path(),
                        "snippet": h.snippet(),
                        "score": h.score(),
                        "line_number": h.line_number(),
                    })
                })
                .collect();
            Ok(Value::Array(v))
        }
        "reindex" => {
            require_mgmt_admin(auth).map_err(RpcAppError::Other)?;
            let filter: Option<String> = match m.get("path") {
                None | Some(Value::Null) => None,
                Some(v) if v.is_null() => None,
                Some(v) => {
                    let s = v
                        .as_str()
                        .ok_or_else(|| Error::InvalidInput("path must be string or null".into()))?;
                    if s.is_empty() || s == "/" {
                        None
                    } else {
                        let n = normalize_path_for_rpc(s)?;
                        canonical_path_segments(&n)?;
                        Some(n)
                    }
                }
            };
            st.db.reindex(filter.as_deref()).await?;
            Ok(Value::Null)
        }
        "head" => {
            let path = rpc_path(&m, "path")?;
            let lines = get_head_lines(&m)?;
            check_read(auth, &path).map_err(RpcAppError::Other)?;
            let fid = st.db.resolve_existing_file_path(&path).await?;
            let text = st.db.document_head(fid, lines).await?;
            Ok(json!({ "text": text }))
        }
        "tail" => {
            let path = rpc_path(&m, "path")?;
            let mode = get_tail_mode_optional(&m, "lines")?;
            check_read(auth, &path).map_err(RpcAppError::Other)?;
            let fid = st.db.resolve_existing_file_path(&path).await?;
            let text = st.db.document_tail(fid, mode).await?;
            Ok(json!({ "text": text }))
        }
        "slice" => {
            let path = rpc_path(&m, "path")?;
            let start = get_slice_line(&m, "start_line", "from_line")?;
            let end = get_slice_line(&m, "end_line", "to_line")?;
            check_read(auth, &path).map_err(RpcAppError::Other)?;
            let fid = st.db.resolve_existing_file_path(&path).await?;
            let text = st.db.document_slice(fid, start, end).await?;
            Ok(json!({ "text": text }))
        }
        "grep" => {
            let path = rpc_path(&m, "path")?;
            let pattern = get_str(&m, "pattern")?;
            check_read(auth, &path).map_err(RpcAppError::Other)?;
            let max_matches = m
                .get("max_matches")
                .and_then(Value::as_u64)
                .map_or(0, |u| usize::try_from(u).unwrap_or(usize::MAX));
            let fid = st.db.resolve_existing_file_path(&path).await?;
            let invert_match = m
                .get("invert_match")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let lines = st
                .db
                .document_grep(fid, pattern, max_matches, invert_match)
                .await?;
            let v: Vec<Value> = lines
                .into_iter()
                .map(|l| json!({ "line": l.line(), "text": l.text() }))
                .collect();
            Ok(Value::Array(v))
        }
        "wc" => {
            let path = rpc_path(&m, "path")?;
            check_read(auth, &path).map_err(RpcAppError::Other)?;
            let fid = st.db.resolve_existing_file_path(&path).await?;
            let w = st.db.document_wc(fid).await?;
            Ok(json!({
                "bytes": w.bytes(),
                "lines": w.lines(),
                "words": w.words(),
                "chars": w.chars(),
            }))
        }
        "stat" => {
            let path = rpc_path(&m, "path")?;
            check_read(auth, &path).map_err(RpcAppError::Other)?;
            let fid = st.db.resolve_existing_file_path(&path).await?;
            let (meta, _parent_path, lines) = st.db.document_stat(fid).await?;
            Ok(json!({
                "id": meta.id().raw(),
                "path": meta.canonical_path(),
                "size_bytes": meta.size_bytes(),
                "line_count": lines,
                "created_at": meta.created_at(),
                "modified_at": meta.modified_at(),
                "accessed_at": meta.accessed_at(),
                "revision": meta.revision(),
            }))
        }
        "test" => {
            if !m.is_empty() {
                return Err(RpcAppError::Other(Error::InvalidInput(
                    "test: no parameters allowed".into(),
                )));
            }
            Ok(test_payload(
                st.process_started_at,
                st.authenticate_api,
                st.oidc.is_some(),
            ))
        }
        "whoami" => {
            if !m.is_empty() {
                return Err(RpcAppError::Other(Error::InvalidInput(
                    "whoami: no parameters allowed".into(),
                )));
            }
            Ok(whoami_json(auth))
        }
        "acl_list" => {
            if !m.is_empty() {
                return Err(RpcAppError::Other(Error::InvalidInput(
                    "acl_list: no parameters allowed".into(),
                )));
            }
            require_mgmt_admin(auth).map_err(RpcAppError::Other)?;
            let rows = st.db.acl_list_rows().await.map_err(RpcAppError::Other)?;
            let v: Vec<Value> = rows
                .into_iter()
                .map(|(name, body)| json!({ "name": name, "body": body }))
                .collect();
            Ok(Value::Array(v))
        }
        "acl_get" => {
            let name = get_str(&m, "name")?;
            require_mgmt_admin(auth).map_err(RpcAppError::Other)?;
            let body = st.db.acl_get_json(name).await.map_err(RpcAppError::Other)?;
            Ok(json!({ "name": name, "body": body }))
        }
        "acl_put" => {
            let name = get_str(&m, "name")?;
            let body = get_str(&m, "body")?;
            require_mgmt_admin(auth).map_err(RpcAppError::Other)?;
            st.db
                .acl_upsert_validated(name, body)
                .await
                .map_err(RpcAppError::Other)?;
            Ok(Value::Null)
        }
        "acl_destroy" => {
            let name = get_str(&m, "name")?;
            require_mgmt_admin(auth).map_err(RpcAppError::Other)?;
            st.db
                .acl_delete_named(name)
                .await
                .map_err(RpcAppError::Other)?;
            Ok(Value::Null)
        }
        "psk_list" => {
            if !m.is_empty() {
                return Err(RpcAppError::Other(Error::InvalidInput(
                    "psk_list: no parameters allowed".into(),
                )));
            }
            require_mgmt_admin(auth).map_err(RpcAppError::Other)?;
            let rows = st.db.psk_list_rows().await.map_err(RpcAppError::Other)?;
            let v: Vec<Value> = rows
                .into_iter()
                .map(|(name, acl_name, key)| {
                    json!({ "name": name, "acl_name": acl_name, "key": key })
                })
                .collect();
            Ok(Value::Array(v))
        }
        "psk_create" => {
            let name = get_str(&m, "name")?;
            let acl_name = get_str(&m, "acl_name")?;
            require_mgmt_admin(auth).map_err(RpcAppError::Other)?;
            let key = generate_access_psk();
            st.db
                .psk_insert(name, acl_name, &key)
                .await
                .map_err(RpcAppError::Other)?;
            Ok(json!({ "name": name, "key": key }))
        }
        "psk_destroy" => {
            let name = get_str(&m, "name")?;
            require_mgmt_admin(auth).map_err(RpcAppError::Other)?;
            st.db
                .psk_delete_named(name)
                .await
                .map_err(RpcAppError::Other)?;
            Ok(Value::Null)
        }
        "describe" => {
            let path = rpc_path(&m, "path")?;
            canonical_path_segments(&path)?;
            match m.get("description") {
                None => {
                    check_read(auth, &path).map_err(RpcAppError::Other)?;
                    let d = st
                        .db
                        .entry_description(&path)
                        .await
                        .map_err(RpcAppError::Other)?;
                    Ok(json!({ "description": d }))
                }
                Some(v) => {
                    check_write(auth, &path).map_err(RpcAppError::Other)?;
                    let s = v.as_str().ok_or_else(|| {
                        RpcAppError::Other(Error::InvalidInput(
                            "describe: param description must be a string when provided".into(),
                        ))
                    })?;
                    let opt = if s.is_empty() { None } else { Some(s) };
                    st.db
                        .set_entry_description(&path, opt)
                        .await
                        .map_err(RpcAppError::Other)?;
                    match st.db.resolve_existing_file_path(&path).await {
                        Ok(fid) => {
                            let meta = st.db.get_document_meta(fid).await?;
                            Ok(json!({ "revision": meta.revision() }))
                        }
                        Err(_) => Ok(json!({ "revision": Value::Null })),
                    }
                }
            }
        }
        _ => Err(RpcAppError::MethodNotFound(format!(
            "unknown method: {method}"
        ))),
    }
}

#[cfg(test)]
mod access_psk_gen_tests {
    use super::generate_access_psk;

    #[test]
    fn psk_is_44_chars_from_mixed_alnum_alphabet() {
        const OK: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
        let mut saw_upper = false;
        let mut saw_lower = false;
        let mut saw_digit = false;
        for _ in 0..48 {
            let s = generate_access_psk();
            assert_eq!(s.len(), 44, "{s:?}");
            for b in s.bytes() {
                assert!(OK.contains(&b), "char {:?} not in alphabet", b as char);
                saw_upper |= b.is_ascii_uppercase();
                saw_lower |= b.is_ascii_lowercase();
                saw_digit |= b.is_ascii_digit();
            }
        }
        assert!(saw_upper && saw_lower && saw_digit);
    }
}

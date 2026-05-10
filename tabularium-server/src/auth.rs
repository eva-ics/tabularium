//! Request ACL resolution — one context per HTTP invocation for the Golden Throne.

use axum::http::HeaderMap;
use serde_json::{Value, json};
use tabularium::resource_path::join_under_directory;
use tabularium::{AuthContext, Error, ListedEntry, SqliteDatabase};

/// Resolved credential mode for REST / JSON-RPC / MCP.
#[derive(Debug, Clone)]
pub(crate) enum RequestAuth {
    /// `[server].authenticate = false` — full access without key; ACL admin APIs open.
    Disabled,
    /// Valid `X-Auth-Key` mapped to an ACL row.
    Authenticated(AuthContext),
}

pub(crate) fn extract_x_auth_key(headers: &HeaderMap) -> tabularium::Result<&str> {
    let Some(raw) = headers.get(axum::http::header::HeaderName::from_static("x-auth-key")) else {
        return Err(Error::Unauthorized("missing X-Auth-Key header".into()));
    };
    let s = raw
        .to_str()
        .map_err(|_| Error::Unauthorized("invalid X-Auth-Key encoding".into()))?;
    let t = s.trim();
    if t.is_empty() {
        return Err(Error::Unauthorized("empty X-Auth-Key".into()));
    }
    Ok(t)
}

pub(crate) async fn resolve_request_auth(
    db: &SqliteDatabase,
    authenticate: bool,
    headers: &HeaderMap,
) -> tabularium::Result<RequestAuth> {
    if !authenticate {
        return Ok(RequestAuth::Disabled);
    }
    let key = extract_x_auth_key(headers)?;
    let ctx = db
        .resolve_auth_key(key)
        .await?
        .ok_or_else(|| Error::Unauthorized("unknown X-Auth-Key".into()))?;
    Ok(RequestAuth::Authenticated(ctx))
}

pub(crate) fn require_mgmt_admin(auth: &RequestAuth) -> tabularium::Result<()> {
    match auth {
        RequestAuth::Disabled => Ok(()),
        RequestAuth::Authenticated(ctx) if ctx.admin() => Ok(()),
        RequestAuth::Authenticated(_) => {
            Err(Error::Forbidden("ACL administration requires admin".into()))
        }
    }
}

pub(crate) fn check_read(auth: &RequestAuth, abs_path: &str) -> tabularium::Result<()> {
    match auth {
        RequestAuth::Disabled => Ok(()),
        RequestAuth::Authenticated(ctx) => ctx.check_read_abs(abs_path),
    }
}

pub(crate) fn check_write(auth: &RequestAuth, abs_path: &str) -> tabularium::Result<()> {
    match auth {
        RequestAuth::Disabled => Ok(()),
        RequestAuth::Authenticated(ctx) => ctx.check_write_abs(abs_path),
    }
}

pub(crate) fn filter_listed_children(
    auth: &RequestAuth,
    dir_abs: &str,
    rows: Vec<ListedEntry>,
) -> Vec<ListedEntry> {
    match auth {
        RequestAuth::Disabled => rows,
        RequestAuth::Authenticated(ctx) if ctx.admin() => rows,
        RequestAuth::Authenticated(ctx) => rows
            .into_iter()
            .filter(|e| {
                let child = join_under_directory(dir_abs, e.name());
                ctx.check_read_abs(&child).is_ok()
            })
            .collect(),
    }
}

pub(crate) fn whoami_json(auth: &RequestAuth) -> Value {
    match auth {
        RequestAuth::Disabled => json!({
            "acl": Value::Null,
            "admin": true,
            "permissions": {
                "allow": { "read": Value::Array(vec![]), "write": Value::Array(vec![]) },
                "deny": { "read": Value::Array(vec![]), "write": Value::Array(vec![]) },
            },
        }),
        RequestAuth::Authenticated(ctx) => ctx.whoami_json(),
    }
}

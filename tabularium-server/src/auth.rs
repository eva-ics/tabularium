//! Request ACL resolution — one context per HTTP invocation for the Golden Throne.

use std::sync::Arc;

use axum::http::HeaderMap;
use serde_json::{Value, json};
use tabularium::resource_path::join_under_directory;
use tabularium::{AuthContext, Error, ListedEntry, SqliteDatabase};

use crate::jwt_assertion::{AssertionRuntime, normalize_groups_from_claim};

/// Resolved credential mode for REST / JSON-RPC / MCP.
#[derive(Debug, Clone)]
pub(crate) enum RequestAuth {
    /// `[server].authenticate = false` — full access without key; ACL admin APIs open.
    Disabled,
    /// Valid `X-Auth-Key` mapped to an ACL row.
    Authenticated(AuthContext),
}

pub(crate) enum AssertionHeader<'a> {
    Absent,
    EmptyOrBadEncoding,
    Token(&'a str),
}

/// Inspect configured assertion header (`[oidc]`). Empty header value is treated as supplied-but-invalid (fail closed).
pub(crate) fn peek_assertion_header<'a>(
    headers: &'a HeaderMap,
    header_name: &axum::http::HeaderName,
) -> AssertionHeader<'a> {
    let Some(raw) = headers.get(header_name) else {
        return AssertionHeader::Absent;
    };
    let Ok(s) = raw.to_str() else {
        return AssertionHeader::EmptyOrBadEncoding;
    };
    let t = s.trim();
    if t.is_empty() {
        AssertionHeader::EmptyOrBadEncoding
    } else {
        AssertionHeader::Token(t)
    }
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
    oidc: Option<&AssertionRuntime>,
) -> tabularium::Result<RequestAuth> {
    if !authenticate {
        return Ok(RequestAuth::Disabled);
    }

    if let Some(rt) = oidc {
        match peek_assertion_header(headers, rt.header_name()) {
            AssertionHeader::Absent => {}
            AssertionHeader::EmptyOrBadEncoding => {
                return Err(Error::Unauthorized(
                    "assertion header present but empty or not valid UTF-8".into(),
                ));
            }
            AssertionHeader::Token(token) => {
                let claims = rt.verify_decode_claims(token)?;
                let groups = normalize_groups_from_claim(
                    &claims,
                    rt.groups_field(),
                    rt.group_name_prefix(),
                )?;
                let mut contexts = Vec::new();
                for g in groups {
                    if let Some(ctx) = db.resolve_acl_named(&g).await? {
                        contexts.push(ctx);
                    }
                }
                if contexts.is_empty() {
                    return Err(Error::Unauthorized(
                        "no ACL matched JWT groups after prefix filtering".into(),
                    ));
                }
                let merged = AuthContext::merge_union(contexts)?;
                return Ok(RequestAuth::Authenticated(merged));
            }
        }
    }

    let key = extract_x_auth_key(headers)?;
    let ctx = db
        .resolve_auth_key(key)
        .await?
        .ok_or_else(|| Error::Unauthorized("unknown X-Auth-Key".into()))?;
    Ok(RequestAuth::Authenticated(ctx))
}

/// [`Arc`] wrapper indirection so REST/MCP/WebSocket share the same resolver without borrow fights.
pub(crate) async fn resolve_request_auth_arc(
    db: &SqliteDatabase,
    authenticate: bool,
    headers: &HeaderMap,
    oidc: Option<&Arc<AssertionRuntime>>,
) -> tabularium::Result<RequestAuth> {
    resolve_request_auth(db, authenticate, headers, oidc.map(|a| a.as_ref())).await
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

/// Directory subtree search (`path` / `dir` limiter): allow when listing would show this directory
/// as a traversal root — same reachability doctrine as [`filter_listed_children`].
pub(crate) fn check_search_directory_traverse(
    auth: &RequestAuth,
    abs_dir: &str,
) -> tabularium::Result<()> {
    match auth {
        RequestAuth::Disabled => Ok(()),
        RequestAuth::Authenticated(ctx) if ctx.admin() => Ok(()),
        RequestAuth::Authenticated(ctx) => {
            if ctx.check_read_abs_for_listing(abs_dir, tabularium::EntryKind::Dir)? {
                Ok(())
            } else {
                Err(tabularium::Error::Forbidden("read denied".into()))
            }
        }
    }
}

pub(crate) fn check_write(auth: &RequestAuth, abs_path: &str) -> tabularium::Result<()> {
    match auth {
        RequestAuth::Disabled => Ok(()),
        RequestAuth::Authenticated(ctx) => ctx.check_write_abs(abs_path),
    }
}

/// If false, parent lacks read but listing is still allowed and rows are filtered (narrow subtree ACL).
pub(crate) fn directory_listing_requires_parent_readable(
    auth: &RequestAuth,
    dir_abs: &str,
) -> bool {
    match auth {
        RequestAuth::Disabled => true,
        RequestAuth::Authenticated(ctx) if ctx.admin() => true,
        RequestAuth::Authenticated(ctx) => ctx.check_read_abs(dir_abs).is_ok(),
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
                ctx.check_read_abs_for_listing(&child, e.kind())
                    .unwrap_or(false)
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

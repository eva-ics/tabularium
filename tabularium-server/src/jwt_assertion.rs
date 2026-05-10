//! Upstream signed JWT assertions — JWKS refresh and local verification for the Throne.

use std::sync::Arc;
use std::time::Duration;

use axum::http::HeaderName;
use jsonwebtoken::jwk::{Jwk, JwkSet};
use jsonwebtoken::{DecodingKey, Validation, decode, decode_header};
use parking_lot::Mutex;
use serde_json::Value;
use tokio::time::MissedTickBehavior;
use tracing::{debug, error};

use crate::config::OidcSection;
use tabularium::{Error, Result};

struct Inner {
    cfg: OidcSection,
    header: HeaderName,
    jwks: Mutex<Option<JwkSet>>,
    client: reqwest::Client,
}

/// JWKS-backed verifier for `[oidc]` assertion tokens (`X-JWT-Assertion` by default).
pub struct AssertionRuntime {
    inner: Arc<Inner>,
}

impl AssertionRuntime {
    /// Loads JWKS immediately (startup fails if unreachable), then refreshes on an interval.
    pub async fn bootstrap(cfg: OidcSection) -> Result<Self> {
        if cfg.key.trim().is_empty() {
            return Err(Error::InvalidInput(
                "[oidc].key must be set when the [oidc] section is present".into(),
            ));
        }
        let header = HeaderName::try_from(cfg.header.trim()).map_err(|_| {
            Error::InvalidInput(format!(
                "invalid [oidc].header value {:?}",
                cfg.header.trim()
            ))
        })?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(cfg.timeout.max(1)))
            .build()
            .map_err(|e| Error::InvalidInput(format!("HTTP client: {e}")))?;
        let initial = fetch_jwks(&client, &cfg.key).await?;
        let inner = Arc::new(Inner {
            cfg: cfg.clone(),
            header,
            jwks: Mutex::new(Some(initial)),
            client,
        });
        let bg = Arc::clone(&inner);
        tokio::spawn(async move {
            refresh_loop(bg).await;
        });
        Ok(Self { inner })
    }

    pub(crate) fn header_name(&self) -> &HeaderName {
        &self.inner.header
    }

    pub(crate) fn groups_field(&self) -> &str {
        self.inner.cfg.groups_field.trim()
    }

    pub(crate) fn group_name_prefix(&self) -> Option<&str> {
        self.inner
            .cfg
            .group_name_prefix
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    /// Validate signature + `exp` / `nbf`; return raw claims JSON (`iss`/`aud` not configurable — canonical `[oidc]` keys only).
    pub(crate) fn verify_decode_claims(&self, token: &str) -> Result<Value> {
        let header =
            decode_header(token).map_err(|e| Error::Unauthorized(format!("jwt header: {e}")))?;
        let validation = validation_for(header.alg);
        let guard = self.inner.jwks.lock();
        let Some(set) = guard.as_ref() else {
            return Err(Error::Unauthorized(
                "JWKS cache empty after sustained fetch failure".into(),
            ));
        };
        let kid = header.kid.as_deref();
        for jwk in ordered_jwks(set, kid) {
            let dk = DecodingKey::from_jwk(jwk)
                .map_err(|e| Error::Unauthorized(format!("invalid JWK: {e}")))?;
            match decode::<Value>(token, &dk, &validation) {
                Ok(t) => return Ok(t.claims),
                Err(e) => {
                    debug!(target: "tabularium_server::oidc", err=%e, "JWK trial decode failed")
                }
            }
        }
        Err(Error::Unauthorized(
            "JWT verification failed for all configured keys".into(),
        ))
    }
}

async fn refresh_loop(inner: Arc<Inner>) {
    let refresh = Duration::from_secs(inner.cfg.refresh.max(1));
    let retry = Duration::from_secs(inner.cfg.retry.max(1));
    let mut int = tokio::time::interval(refresh);
    int.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        int.tick().await;
        loop {
            match fetch_jwks(&inner.client, &inner.cfg.key).await {
                Ok(set) => {
                    *inner.jwks.lock() = Some(set);
                    debug!(target: "tabularium_server::oidc", path = %inner.cfg.key, "JWKS refreshed");
                    break;
                }
                Err(e) => {
                    error!(target: "tabularium_server::oidc", path = %inner.cfg.key, err=%e, "JWKS refresh failed");
                    tokio::time::sleep(retry).await;
                }
            }
        }
    }
}

async fn fetch_jwks(client: &reqwest::Client, path: &str) -> Result<JwkSet> {
    let data = if path.starts_with("http://") || path.starts_with("https://") {
        let res = client
            .get(path)
            .send()
            .await
            .map_err(|e| Error::Io(format!("JWKS fetch {path}: {e}")))?;
        if !res.status().is_success() {
            return Err(Error::Io(format!(
                "JWKS fetch {path}: HTTP {}",
                res.status()
            )));
        }
        res.text()
            .await
            .map_err(|e| Error::Io(format!("JWKS fetch {path}: {e}")))?
    } else {
        tokio::fs::read_to_string(path)
            .await
            .map_err(|e| Error::Io(format!("JWKS read {}: {e}", path)))?
    };
    let set: JwkSet = serde_json::from_str(&data)
        .map_err(|e| Error::InvalidInput(format!("JWKS JSON {path}: {e}")))?;
    if set.keys.is_empty() {
        return Err(Error::InvalidInput(format!(
            "JWKS at {path} contains no keys"
        )));
    }
    Ok(set)
}

fn ordered_jwks<'a>(set: &'a JwkSet, kid: Option<&str>) -> Vec<&'a Jwk> {
    let mut primary = Vec::new();
    let mut rest = Vec::new();
    for j in &set.keys {
        let matches = kid.is_some_and(|k| j.common.key_id.as_deref() == Some(k));
        if matches {
            primary.push(j);
        } else {
            rest.push(j);
        }
    }
    primary.into_iter().chain(rest).collect()
}

fn validation_for(alg: jsonwebtoken::Algorithm) -> Validation {
    let mut v = Validation::new(alg);
    v.validate_nbf = true;
    v.validate_aud = false;
    v.set_required_spec_claims(&["exp"]);
    v
}

/// Normalize group names from JWT claims using optional prefix filter (STC assertion doctrine).
pub(crate) fn normalize_groups_from_claim(
    claims: &Value,
    field: &str,
    prefix: Option<&str>,
) -> Result<Vec<String>> {
    if field.is_empty() {
        return Err(Error::Unauthorized(
            "[oidc].groups_field must not be empty".into(),
        ));
    }
    let Some(raw) = claims.get(field) else {
        return Err(Error::Unauthorized(format!(
            "missing JWT claim `{field}` for ACL mapping"
        )));
    };
    let mut groups = Vec::new();
    match raw {
        Value::Array(a) => {
            for v in a {
                let s = v.as_str().ok_or_else(|| {
                    Error::Unauthorized(format!(
                        "JWT `{field}` entries must be strings (got non-string array element)"
                    ))
                })?;
                groups.push(s.to_string());
            }
        }
        Value::String(s) => {
            for part in s.split(',') {
                let t = part.trim();
                if !t.is_empty() {
                    groups.push(t.to_string());
                }
            }
        }
        _ => {
            return Err(Error::Unauthorized(format!(
                "JWT `{field}` must be a JSON array of strings or a comma-separated string"
            )));
        }
    }
    let mut out = Vec::new();
    for g in groups {
        let g = g.trim();
        if g.is_empty() {
            continue;
        }
        match prefix {
            None => out.push(g.to_string()),
            Some(p) => {
                if let Some(stripped) = g.strip_prefix(p) {
                    let s = stripped.trim();
                    if !s.is_empty() {
                        out.push(s.to_string());
                    }
                }
            }
        }
    }
    Ok(out)
}

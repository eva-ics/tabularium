//! Stage-1 ACL doctrine — default deny, deny overrides allow, `path/*` matches strict descendants only.

use serde::{Deserialize, Serialize};

use crate::resource_path::canonical_path_segments;
use crate::validation::validate_entity_name;
use crate::{Error, Result};

/// Wire / persisted JSON shape (`admin`, `allow`, `deny`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AclBodyJson {
    pub admin: bool,
    pub allow: AclSideJson,
    pub deny: AclSideJson,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AclSideJson {
    #[serde(default)]
    pub read: Vec<String>,
    #[serde(default)]
    pub write: Vec<String>,
}

/// Resolved credential for one request — Omnissiah sees all once per invocation.
#[derive(Debug, Clone)]
pub struct AuthContext {
    acl_name: String,
    admin: bool,
    allow: AclSideJson,
    deny: AclSideJson,
}

impl AuthContext {
    pub fn new(acl_name: String, body: AclBodyJson) -> Self {
        Self {
            acl_name,
            admin: body.admin,
            allow: body.allow,
            deny: body.deny,
        }
    }

    pub fn acl_name(&self) -> &str {
        &self.acl_name
    }

    pub fn admin(&self) -> bool {
        self.admin
    }

    pub fn allow(&self) -> &AclSideJson {
        &self.allow
    }

    pub fn deny(&self) -> &AclSideJson {
        &self.deny
    }

    /// `whoami` payload fragment (`permissions` mirrors stored JSON).
    pub fn whoami_json(&self) -> serde_json::Value {
        serde_json::json!({
            "acl": self.acl_name,
            "admin": self.admin,
            "permissions": {
                "allow": { "read": self.allow.read, "write": self.allow.write },
                "deny": { "read": self.deny.read, "write": self.deny.write },
            }
        })
    }

    pub fn check_read_abs(&self, abs_path: &str) -> Result<()> {
        if self.admin {
            return Ok(());
        }
        let rel = absolute_to_rel(abs_path)?;
        check_side(&rel, &self.deny.read, &self.allow.read, "read denied")
    }

    pub fn check_write_abs(&self, abs_path: &str) -> Result<()> {
        if self.admin {
            return Ok(());
        }
        let rel = absolute_to_rel(abs_path)?;
        check_side(&rel, &self.deny.write, &self.allow.write, "write denied")
    }
}

fn absolute_to_rel(abs_path: &str) -> Result<String> {
    canonical_path_segments(abs_path)?;
    if abs_path == "/" {
        return Ok(String::new());
    }
    Ok(abs_path.trim_start_matches('/').to_string())
}

fn check_side(rel: &str, deny: &[String], allow: &[String], forbidden_msg: &str) -> Result<()> {
    for rule in deny {
        validate_rule_pattern(rule)?;
        if rule_matches(rel, rule) {
            return Err(Error::Forbidden(forbidden_msg.into()));
        }
    }
    let mut ok = false;
    for rule in allow {
        validate_rule_pattern(rule)?;
        if rule_matches(rel, rule) {
            ok = true;
            break;
        }
    }
    if ok {
        Ok(())
    } else {
        Err(Error::Forbidden(forbidden_msg.into()))
    }
}

/// Validate and parse ACL JSON from wire / DB. Bad schema must not persist.
pub fn parse_acl_json(raw: &str) -> Result<AclBodyJson> {
    let v: AclBodyJson =
        serde_json::from_str(raw).map_err(|e| Error::InvalidInput(e.to_string()))?;
    validate_acl_body(&v)?;
    Ok(v)
}

fn validate_acl_body(body: &AclBodyJson) -> Result<()> {
    if body.admin {
        return Ok(());
    }
    for r in &body.allow.read {
        validate_rule_pattern(r)?;
    }
    for r in &body.allow.write {
        validate_rule_pattern(r)?;
    }
    for r in &body.deny.read {
        validate_rule_pattern(r)?;
    }
    for r in &body.deny.write {
        validate_rule_pattern(r)?;
    }
    Ok(())
}

fn validate_rule_pattern(rule: &str) -> Result<()> {
    let t = rule.trim();
    // Empty pattern matches only the librarium root directory (`/` → relative "").
    if t.is_empty() {
        return Ok(());
    }
    let path_part = if let Some(base) = t.strip_suffix("/*") {
        if base.is_empty() {
            return Err(Error::InvalidInput(
                "ACL subtree rule must not be bare /*".into(),
            ));
        }
        base
    } else {
        t
    };
    for seg in path_part.split('/') {
        if seg.is_empty() {
            return Err(Error::InvalidInput(
                "ACL rule path must not contain empty segments".into(),
            ));
        }
        validate_entity_name(seg)?;
    }
    Ok(())
}

fn rule_matches(rel: &str, rule: &str) -> bool {
    let rule = rule.trim();
    if rule.is_empty() {
        return rel.is_empty();
    }
    if let Some(base) = rule.strip_suffix("/*") {
        descendant_match(rel, base)
    } else {
        rel == rule
    }
}

/// `base/*` matches strict descendants only (not `base` itself).
fn descendant_match(rel: &str, base: &str) -> bool {
    if base.is_empty() {
        return !rel.is_empty();
    }
    let prefix = format!("{base}/");
    rel.starts_with(&prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(admin: bool, allow_r: &[&str], deny_r: &[&str]) -> AuthContext {
        AuthContext::new(
            "t".into(),
            AclBodyJson {
                admin,
                allow: AclSideJson {
                    read: allow_r.iter().map(|s| (*s).to_string()).collect(),
                    write: vec![],
                },
                deny: AclSideJson {
                    read: deny_r.iter().map(|s| (*s).to_string()).collect(),
                    write: vec![],
                },
            },
        )
    }

    #[test]
    fn subtree_does_not_match_self() {
        let c = ctx(false, &["foo/*"], &[]);
        assert!(c.check_read_abs("/foo/bar").is_ok());
        assert!(c.check_read_abs("/foo").is_err());
    }

    #[test]
    fn deny_beats_allow() {
        let c = AuthContext::new(
            "t".into(),
            AclBodyJson {
                admin: false,
                allow: AclSideJson {
                    read: vec!["foo/*".into()],
                    write: vec![],
                },
                deny: AclSideJson {
                    read: vec!["foo/bar".into()],
                    write: vec![],
                },
            },
        );
        assert!(c.check_read_abs("/foo/bar").is_err());
        assert!(c.check_read_abs("/foo/baz").is_ok());
    }

    #[test]
    fn default_deny() {
        let c = ctx(false, &[], &[]);
        assert!(c.check_read_abs("/any").is_err());
    }

    #[test]
    fn admin_skips() {
        let c = ctx(true, &[], &[]);
        assert!(c.check_read_abs("/x/y").is_ok());
        assert!(c.check_write_abs("/x/y").is_ok());
    }
}

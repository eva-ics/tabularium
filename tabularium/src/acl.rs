//! Stage-1 ACL doctrine — default deny, deny overrides allow, `path/*` matches strict descendants only.
//!
//! Allow/deny patterns are **absolute** librarium paths (`/`, `/docs/foo`, `/vault/*`). Relative strings
//! like `docs/foo` are rejected at validation.

use serde::{Deserialize, Serialize};

use crate::resource_path::canonical_path_segments;
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

    /// Merge matched ACLs into one principal: **union** of allow/deny rules; any `admin` wins.
    /// Deny still overrides allow on each side ([`Self::check_read_abs`], [`Self::check_write_abs`]).
    pub fn merge_union(mut contexts: Vec<Self>) -> Result<Self> {
        if contexts.is_empty() {
            return Err(Error::InvalidInput(
                "ACL merge requires at least one context".into(),
            ));
        }
        contexts.sort_by(|a, b| a.acl_name().cmp(b.acl_name()));
        let admin = contexts.iter().any(AuthContext::admin);
        let mut allow_read: Vec<String> = contexts
            .iter()
            .flat_map(|c| c.allow.read.iter().cloned())
            .collect();
        let mut allow_write: Vec<String> = contexts
            .iter()
            .flat_map(|c| c.allow.write.iter().cloned())
            .collect();
        let mut deny_read: Vec<String> = contexts
            .iter()
            .flat_map(|c| c.deny.read.iter().cloned())
            .collect();
        let mut deny_write: Vec<String> = contexts
            .iter()
            .flat_map(|c| c.deny.write.iter().cloned())
            .collect();
        sort_dedup_strings(&mut allow_read);
        sort_dedup_strings(&mut allow_write);
        sort_dedup_strings(&mut deny_read);
        sort_dedup_strings(&mut deny_write);
        let mut names: Vec<String> = contexts.iter().map(|c| c.acl_name().to_string()).collect();
        names.dedup();
        let acl_name = names.join(",");
        Ok(Self::new(
            acl_name,
            AclBodyJson {
                admin,
                allow: AclSideJson {
                    read: allow_read,
                    write: allow_write,
                },
                deny: AclSideJson {
                    read: deny_read,
                    write: deny_write,
                },
            },
        ))
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

    /// Parent [`list_directory`] rows: files require [`check_read_abs`]. Directories also pass when some
    /// **allow** read rule reaches this directory or strictly beneath it (e.g. `/vault/*` exposes `/vault`).
    pub fn check_read_abs_for_listing(
        &self,
        abs_path: &str,
        kind: crate::EntryKind,
    ) -> Result<bool> {
        if self.admin {
            return Ok(true);
        }
        if self.check_read_abs(abs_path).is_ok() {
            return Ok(true);
        }
        if kind != crate::EntryKind::Dir {
            return Ok(false);
        }
        let rel = absolute_to_rel(abs_path)?;
        Ok(self.allow_reaches_directory_for_listing(&rel))
    }

    fn allow_reaches_directory_for_listing(&self, child_rel: &str) -> bool {
        for rule in &self.allow.read {
            if validate_rule_pattern(rule).is_err() {
                continue;
            }
            if rule_implies_listed_directory_visible(child_rel, rule) {
                return true;
            }
        }
        false
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

fn sort_dedup_strings(v: &mut Vec<String>) {
    v.sort();
    v.dedup();
}

/// Whether an allow rule grants listing visibility for directory `child_rel` (relative path, `""` = root).
fn rule_implies_listed_directory_visible(child_rel: &str, rule: &str) -> bool {
    let rule = rule.trim();
    if rule.is_empty() {
        return false;
    }
    let anchor_rel = if let Some(base) = rule.strip_suffix("/*") {
        let base = base.trim();
        if base.is_empty() || base == "/" {
            return false;
        }
        match absolute_to_rel(base) {
            Ok(r) => r,
            Err(_) => return false,
        }
    } else {
        match absolute_to_rel(rule) {
            Ok(r) => r,
            Err(_) => return false,
        }
    };
    child_rel == anchor_rel.as_str()
        || anchor_rel.starts_with(&format!("{child_rel}/"))
        || child_rel.starts_with(&format!("{anchor_rel}/"))
}

fn validate_rule_pattern(rule: &str) -> Result<()> {
    let t = rule.trim();
    if t.is_empty() {
        return Err(Error::InvalidInput(
            "ACL rule must be absolute (start with /); use / for librarium root".into(),
        ));
    }
    if let Some(base) = t.strip_suffix("/*") {
        let base = base.trim();
        if base.is_empty() || base == "/" {
            return Err(Error::InvalidInput(
                "ACL subtree rule must not be bare /*".into(),
            ));
        }
        canonical_path_segments(base)?;
    } else {
        canonical_path_segments(t)?;
    }
    Ok(())
}

fn rule_matches(rel: &str, rule: &str) -> bool {
    let rule = rule.trim();
    if rule.is_empty() {
        return false;
    }
    if let Some(base_abs) = rule.strip_suffix("/*") {
        let base_abs = base_abs.trim();
        if base_abs.is_empty() || base_abs == "/" {
            return false;
        }
        let Ok(base_rel) = absolute_to_rel(base_abs) else {
            return false;
        };
        descendant_match(rel, &base_rel)
    } else {
        let Ok(rule_rel) = absolute_to_rel(rule) else {
            return false;
        };
        rel == rule_rel
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
        let c = ctx(false, &["/foo/*"], &[]);
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
                    read: vec!["/foo/*".into()],
                    write: vec![],
                },
                deny: AclSideJson {
                    read: vec!["/foo/bar".into()],
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

    #[test]
    fn merge_union_unions_allows_any_admin() {
        let a = AuthContext::new(
            "a".into(),
            AclBodyJson {
                admin: false,
                allow: AclSideJson {
                    read: vec!["/foo".into()],
                    write: vec![],
                },
                deny: AclSideJson {
                    read: vec![],
                    write: vec![],
                },
            },
        );
        let b = AuthContext::new(
            "b".into(),
            AclBodyJson {
                admin: false,
                allow: AclSideJson {
                    read: vec!["/bar".into()],
                    write: vec![],
                },
                deny: AclSideJson {
                    read: vec![],
                    write: vec![],
                },
            },
        );
        let m = AuthContext::merge_union(vec![a, b]).unwrap();
        assert!(!m.admin());
        assert!(m.check_read_abs("/foo").is_ok());
        assert!(m.check_read_abs("/bar").is_ok());
        assert!(m.check_read_abs("/baz").is_err());
    }

    #[test]
    fn root_rule_slash_matches_only_root() {
        let c = ctx(false, &["/"], &[]);
        assert!(c.check_read_abs("/").is_ok());
        assert!(c.check_read_abs("/x").is_err());
    }

    #[test]
    fn parse_acl_rejects_unrooted_rule() {
        let raw = r#"{"admin":false,"allow":{"read":["rel/path"],"write":[]},"deny":{"read":[],"write":[]}}"#;
        assert!(parse_acl_json(raw).is_err());
    }

    #[test]
    fn parse_acl_accepts_absolute_rules() {
        let raw = r#"{"admin":false,"allow":{"read":["/","/a/b"],"write":[]},"deny":{"read":[],"write":[]}}"#;
        assert!(parse_acl_json(raw).is_ok());
    }

    #[test]
    fn listing_includes_dir_when_only_subtree_allow() {
        let c = ctx(false, &["/test/*"], &[]);
        assert!(c.check_read_abs("/test").is_err());
        assert!(
            c.check_read_abs_for_listing("/test", crate::EntryKind::Dir)
                .unwrap()
        );
        assert!(
            c.check_read_abs_for_listing("/test/inside.txt", crate::EntryKind::File)
                .unwrap()
        );
        assert!(
            !c.check_read_abs_for_listing("/other.txt", crate::EntryKind::File)
                .unwrap()
        );
    }

    #[test]
    fn listing_includes_intermediate_dirs_for_deeper_subtree() {
        let c = ctx(false, &["/vault/deep/*"], &[]);
        assert!(
            c.check_read_abs_for_listing("/vault", crate::EntryKind::Dir)
                .unwrap()
        );
        assert!(
            c.check_read_abs_for_listing("/vault/deep", crate::EntryKind::Dir)
                .unwrap()
        );
        assert!(
            !c.check_read_abs_for_listing("/other", crate::EntryKind::Dir)
                .unwrap()
        );
    }

    #[test]
    fn subtree_allow_matches_deep_paths() {
        let c = ctx(false, &["/test/*"], &[]);
        assert!(c.check_read_abs("/test/file.txt").is_ok());
        assert!(c.check_read_abs("/test/a/b/c.txt").is_ok());
    }

    #[test]
    fn exact_allow_blocks_sibling_file() {
        let c = ctx(false, &["/test/file.txt"], &[]);
        assert!(c.check_read_abs("/test/file.txt").is_ok());
        assert!(c.check_read_abs("/test/other.txt").is_err());
    }

    #[test]
    fn subtree_allow_blocks_sibling_branch() {
        let c = ctx(false, &["/test/*"], &[]);
        assert!(c.check_read_abs("/other/file.txt").is_err());
    }

    #[test]
    fn parse_acl_rejects_bare_subtree_glob() {
        let raw =
            r#"{"admin":false,"allow":{"read":["/*"],"write":[]},"deny":{"read":[],"write":[]}}"#;
        assert!(parse_acl_json(raw).is_err());
    }
}

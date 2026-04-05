//! Canonical absolute paths: `/`, `/a`, `/a/b`, `/a/b.txt` — no backslashes, no `//`, no trailing `/` (except root).

use std::path::Path;

use crate::validation::validate_entity_name;
use crate::{Error, Result};

/// Rejects `\` in any public path string.
pub fn assert_no_backslash(path: &str) -> Result<()> {
    if path.contains('\\') {
        return Err(Error::InvalidInput(
            "path must not contain backslash".into(),
        ));
    }
    Ok(())
}

/// Normalizes and validates an absolute path; returns path segments under root (empty for `/`).
pub fn canonical_path_segments(path: &str) -> Result<Vec<String>> {
    assert_no_backslash(path)?;
    let path = path.trim();
    if !path.starts_with('/') {
        return Err(Error::InvalidInput(
            "path must be absolute (start with /)".into(),
        ));
    }
    if path == "/" {
        return Ok(Vec::new());
    }
    if path.ends_with('/') {
        return Err(Error::InvalidInput(
            "non-root path must not end with /".into(),
        ));
    }
    let inner = path.trim_start_matches('/');
    let mut out = Vec::new();
    for seg in inner.split('/') {
        if seg.is_empty() {
            return Err(Error::InvalidInput("empty path segment".into()));
        }
        validate_entity_name(seg)?;
        out.push(seg.to_string());
    }
    Ok(out)
}

/// Lexically normalize a UTF-8 absolute tabularium path: collapse `//`, drop `.`, resolve `..` under root, validate segments.
///
/// No filesystem access — suitable for the RPC wire boundary.
pub fn normalize_path_for_rpc(path: impl AsRef<Path>) -> Result<String> {
    let s = path
        .as_ref()
        .to_str()
        .ok_or_else(|| Error::InvalidInput("path must be valid UTF-8".into()))?;
    let s = s.trim();
    if s.is_empty() {
        return Err(Error::InvalidInput("path must not be empty".into()));
    }
    assert_no_backslash(s)?;
    let with_leading = if s.starts_with('/') {
        s.to_string()
    } else {
        format!("/{}", s.trim_start_matches('/'))
    };
    lexical_normalize_absolute_path(&with_leading)
}

fn lexical_normalize_absolute_path(path: &str) -> Result<String> {
    debug_assert!(path.starts_with('/'));
    let inner = path.trim_start_matches('/');
    let mut stack: Vec<&str> = Vec::new();
    for seg in inner.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            if stack.pop().is_none() {
                return Err(Error::InvalidInput("path escapes above root (..)".into()));
            }
            continue;
        }
        validate_entity_name(seg)?;
        stack.push(seg);
    }
    if stack.is_empty() {
        Ok("/".to_string())
    } else {
        Ok(format!("/{}", stack.join("/")))
    }
}

/// User-facing path: add a leading `/` when absent (`notes/a` → `/notes/a`).
pub fn normalize_user_path(path: &str) -> Result<String> {
    let t = path.trim();
    if t.is_empty() {
        return Err(Error::InvalidInput("path must not be empty".into()));
    }
    assert_no_backslash(t)?;
    if t.starts_with('/') {
        Ok(t.to_string())
    } else {
        Ok(format!("/{t}"))
    }
}

/// Parent directory path and final segment name for a non-root path (file or directory).
pub fn parent_and_final_name(path: &str) -> Result<(String, String)> {
    let segs = canonical_path_segments(path)?;
    if segs.is_empty() {
        return Err(Error::InvalidInput(
            "path must name an entry under root (not / alone)".into(),
        ));
    }
    if segs.len() == 1 {
        return Ok((String::from("/"), segs[0].clone()));
    }
    let name = segs[segs.len() - 1].clone();
    let parent = format!("/{}", segs[..segs.len() - 1].join("/"));
    Ok((parent, name))
}

#[cfg(test)]
mod normalize_rpc_tests {
    use std::path::Path;

    use super::normalize_path_for_rpc;

    #[test]
    fn adds_leading_slash_and_collapses_dot_segments() {
        assert_eq!(normalize_path_for_rpc("a/b").unwrap(), "/a/b");
        assert_eq!(normalize_path_for_rpc("/a//b/./c").unwrap(), "/a/b/c");
    }

    #[test]
    fn resolves_dotdot_under_root() {
        assert_eq!(normalize_path_for_rpc("/a/b/../c").unwrap(), "/a/c");
    }

    #[test]
    fn rejects_escape_above_root() {
        assert!(normalize_path_for_rpc("/../a").is_err());
    }

    #[test]
    fn path_buf_and_str() {
        assert_eq!(normalize_path_for_rpc(Path::new("/x/y")).unwrap(), "/x/y");
    }
}

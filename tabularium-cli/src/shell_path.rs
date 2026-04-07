//! Interactive-shell path resolution: fold `.` / `..` / `./` against `shell_cwd` before RPC.

use tabularium::resource_path::assert_no_backslash;
use tabularium::validate_entity_name;

fn cwd_to_segments(cwd: Option<&str>) -> Vec<String> {
    let Some(c) = cwd else {
        return Vec::new();
    };
    let c = c.trim().trim_start_matches('/').trim_end_matches('/');
    if c.is_empty() {
        Vec::new()
    } else {
        c.split('/').map(str::to_string).collect()
    }
}

fn fold_parts(base: &mut Vec<String>, parts: &[&str]) -> Result<(), String> {
    for p in parts {
        match *p {
            "." => {}
            ".." => {
                base.pop()
                    .ok_or_else(|| "path escapes repository root".to_string())?;
            }
            seg => {
                validate_entity_name(seg).map_err(|e| e.to_string())?;
                base.push(seg.to_string());
            }
        }
    }
    Ok(())
}

fn resolve_absolute_to_segments(path: &str) -> Result<Vec<String>, String> {
    assert_no_backslash(path).map_err(|e| e.to_string())?;
    let trimmed = path.trim();
    if !trimmed.starts_with('/') {
        return Err("internal: expected absolute path".into());
    }
    let trimmed = trimmed.trim_end_matches('/');
    let inner = trimmed
        .strip_prefix('/')
        .unwrap_or("")
        .trim_start_matches('/');
    let parts: Vec<&str> = inner.split('/').filter(|p| !p.is_empty()).collect();
    let mut base = Vec::new();
    fold_parts(&mut base, &parts)?;
    Ok(base)
}

fn resolve_relative_to_segments(rel: &str, shell_cwd: Option<&str>) -> Result<Vec<String>, String> {
    assert_no_backslash(rel).map_err(|e| e.to_string())?;
    let rel = rel.trim_end_matches('/').trim();
    let parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
    let mut base = cwd_to_segments(shell_cwd);
    fold_parts(&mut base, &parts)?;
    Ok(base)
}

fn doc_path_merge_with_cwd(rel: &str) -> bool {
    let rel = rel.trim_end_matches('/').trim();
    !rel.is_empty()
}

/// `true` for `ls other` style names: one root-level segment, no dot semantics.
pub(crate) fn shell_ls_bare_root_relative(p: &str) -> bool {
    let p = p.trim().trim_end_matches('/');
    !p.is_empty()
        && p != "/"
        && p != "."
        && p != ".."
        && !p.starts_with('/')
        && !p.contains('/')
        && !p.starts_with("./")
        && !p.starts_with("../")
}

fn finish_doc_path_from_segments(base: Vec<String>) -> Result<String, String> {
    let out = base.join("/");
    if out.is_empty() {
        return Err("path resolves to the repository root".into());
    }
    Ok(out)
}

/// Server-side path without leading `/` (RPC layer adds `/` via `normalize_user_path`).
pub(crate) fn resolve_shell_doc_path(
    path: &str,
    shell_cwd: Option<&str>,
) -> Result<String, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("path must not be empty".into());
    }
    if path.starts_with('/') {
        let segs = resolve_absolute_to_segments(path)?;
        return finish_doc_path_from_segments(segs);
    }
    let trimmed = path.trim_end_matches('/');
    let rel = trimmed;
    let merge_cwd = doc_path_merge_with_cwd(rel);
    let shell_for_rel = if merge_cwd { shell_cwd } else { None };
    let segs = resolve_relative_to_segments(rel, shell_for_rel)?;
    finish_doc_path_from_segments(segs)
}

/// Tree scope: `/`, or segments without leading `/` (except root sentinel).
pub(crate) fn resolve_shell_tree_scope(
    dir: &str,
    shell_cwd: Option<&str>,
) -> Result<String, String> {
    let dir = dir.trim();
    if dir.is_empty() {
        return Err("directory path must not be empty".into());
    }
    if dir == "/" {
        return Ok("/".to_string());
    }
    let segs = if dir.starts_with('/') {
        resolve_absolute_to_segments(dir)?
    } else {
        resolve_relative_to_segments(dir, shell_cwd)?
    };
    Ok(if segs.is_empty() {
        "/".to_string()
    } else {
        segs.join("/")
    })
}

pub(crate) fn shell_path_has_glob(path: &str) -> bool {
    path.chars().any(|c| matches!(c, '*' | '?' | '['))
}

pub(crate) fn resolve_shell_rm_path(
    path: &str,
    recursive: bool,
    shell_cwd: Option<&str>,
) -> Result<String, String> {
    if shell_path_has_glob(path) {
        if let Some(abs) = path.strip_prefix('/') {
            return Ok(abs.trim_end_matches('/').to_string());
        }
        let trimmed = path.trim_end_matches('/');
        if recursive || trimmed.contains('/') {
            return Ok(trimmed.to_string());
        }
        Ok(match shell_cwd {
            Some(c) => format!("{c}/{trimmed}"),
            None => trimmed.to_string(),
        })
    } else {
        resolve_shell_doc_path(path, shell_cwd)
    }
}

pub(crate) fn resolve_ls_directory(
    directory: Option<&str>,
    shell_cwd: Option<&str>,
) -> Result<Option<String>, String> {
    match directory.map(str::trim) {
        None => Ok(shell_cwd.map(str::to_string)),
        Some(p) if shell_ls_bare_root_relative(p) => {
            let p = p.trim_end_matches('/');
            Ok(Some(match shell_cwd {
                Some(c) => {
                    let c = c.trim_end_matches('/');
                    if c.is_empty() {
                        p.to_string()
                    } else {
                        format!("{c}/{p}")
                    }
                }
                None => p.to_string(),
            }))
        }
        Some(p) => {
            let r = resolve_shell_tree_scope(p, shell_cwd)?;
            Ok(Some(if r == "/" { "/".to_string() } else { r }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_single_segment_uses_cwd() {
        assert_eq!(
            resolve_shell_doc_path("doc", Some("notes")).unwrap(),
            "notes/doc"
        );
    }

    #[test]
    fn doc_multi_segment_uses_cwd() {
        assert_eq!(
            resolve_shell_doc_path("x/doc", Some("n")).unwrap(),
            "n/x/doc"
        );
    }

    #[test]
    fn doc_dot_slash_against_cwd() {
        assert_eq!(resolve_shell_doc_path("./doc", Some("n")).unwrap(), "n/doc");
    }

    #[test]
    fn tree_trailing_slash_relative() {
        assert_eq!(
            resolve_shell_tree_scope("sub/", Some("eva")).unwrap(),
            "eva/sub"
        );
    }

    #[test]
    fn ls_bare_glob_prepends_cwd() {
        assert_eq!(
            resolve_ls_directory(Some("*.md"), Some("notes")).unwrap(),
            Some("notes/*.md".to_string())
        );
    }

    #[test]
    fn ls_bare_non_glob_prepends_cwd() {
        assert_eq!(
            resolve_ls_directory(Some("report"), Some("projects")).unwrap(),
            Some("projects/report".to_string())
        );
    }

    #[test]
    fn ls_bare_no_cwd_is_single_segment() {
        assert_eq!(
            resolve_ls_directory(Some("report"), None).unwrap(),
            Some("report".to_string())
        );
    }
}

//! Final-segment glob expansion and directory detection for path-oriented RPC/CLI.

use globset::{GlobBuilder, GlobMatcher};

use crate::EntryKind;
use crate::db::{Database, Storage};
use crate::resource_path::{join_under_directory, parent_and_final_name};
use crate::{Error, Result};

/// Whether `segment` contains glob metacharacters.
pub fn path_has_glob_metachar(segment: &str) -> bool {
    segment.chars().any(|c| matches!(c, '*' | '?' | '['))
}

/// Compile a glob that matches a single path segment name (no `/` in pattern).
pub fn compile_name_glob(pat: &str) -> Result<GlobMatcher> {
    let matcher = GlobBuilder::new(pat)
        .literal_separator(true)
        .build()
        .map_err(|e| Error::InvalidInput(format!("invalid glob pattern: {e}")))?
        .compile_matcher();
    Ok(matcher)
}

/// Split a normalized non-root path into parent directory and final segment when the final segment has glob metacharacters.
pub fn parent_and_glob_pattern(normalized_path: &str) -> Result<Option<(String, String)>> {
    if normalized_path == "/" {
        return Ok(None);
    }
    let (parent, last) = parent_and_final_name(normalized_path)?;
    if !path_has_glob_metachar(&last) {
        return Ok(None);
    }
    if path_has_glob_metachar(&parent) {
        return Err(Error::InvalidInput(
            "wildcards are only allowed in the final path segment".into(),
        ));
    }
    Ok(Some((parent, last)))
}

/// Collect every file path under `dir_path` (depth-first, sorted siblings).
pub async fn collect_files_recursive<S: Storage>(
    db: &Database<S>,
    dir_path: &str,
    out: &mut Vec<String>,
) -> Result<()> {
    let entries = db.list_directory(dir_path).await?;
    for e in entries {
        let full = join_under_directory(dir_path, e.name());
        if e.kind() == EntryKind::File {
            out.push(full);
        } else {
            Box::pin(collect_files_recursive(db, &full, out)).await?;
        }
    }
    Ok(())
}

/// Expand final-segment glob; with `recursive`, matching directories contribute all files in their subtrees.
pub async fn expand_final_segment_glob_files_opts<S: Storage>(
    db: &Database<S>,
    normalized_path: &str,
    recursive: bool,
) -> Result<Option<Vec<String>>> {
    let Some((parent, pattern)) = parent_and_glob_pattern(normalized_path)? else {
        return Ok(None);
    };
    let entries = db.list_directory(&parent).await?;
    let matcher = compile_name_glob(&pattern)?;
    let mut paths: Vec<String> = Vec::new();
    let mut matched_dirs = 0usize;
    for e in entries {
        if !matcher.is_match(e.name()) {
            continue;
        }
        let full = join_under_directory(&parent, e.name());
        match e.kind() {
            EntryKind::File => paths.push(full),
            EntryKind::Dir if recursive => {
                collect_files_recursive(db, &full, &mut paths).await?;
            }
            EntryKind::Dir => {
                matched_dirs += 1;
            }
        }
    }
    paths.sort();
    if paths.is_empty() {
        if matched_dirs > 0 && !recursive {
            return Err(Error::InvalidInput(format!(
                "wc: no files match {normalized_path} ({matched_dirs} director{} matched; use -r/--recursive to count files under them)",
                if matched_dirs == 1 { "y" } else { "ies" }
            )));
        }
        return Err(Error::InvalidInput(format!(
            "wc: no files match {normalized_path}"
        )));
    }
    Ok(Some(paths))
}

/// Whether `path` resolves to an existing directory.
pub async fn path_is_directory<S: Storage>(db: &Database<S>, path: &str) -> Result<bool> {
    Ok(db.resolve_directory_path(path).await.is_ok())
}

/// Resolve `wc` targets: glob expansion, optional recursive directory walk, or a single file path.
pub async fn resolve_wc_file_paths<S: Storage>(
    db: &Database<S>,
    normalized_path: &str,
    recursive: bool,
) -> Result<Vec<String>> {
    if let Some(paths) =
        expand_final_segment_glob_files_opts(db, normalized_path, recursive).await?
    {
        return Ok(paths);
    }
    if path_is_directory(db, normalized_path).await? {
        if recursive {
            let mut paths = Vec::new();
            collect_files_recursive(db, normalized_path, &mut paths).await?;
            paths.sort();
            if paths.is_empty() {
                return Err(Error::InvalidInput(format!(
                    "no files under {normalized_path}"
                )));
            }
            return Ok(paths);
        }
        return Err(Error::InvalidInput(format!(
            "wc: {normalized_path}: is a directory (use -r/--recursive to count files under it)"
        )));
    }
    Ok(vec![normalized_path.to_string()])
}

#[cfg(test)]
mod tests {
    use super::{compile_name_glob, parent_and_glob_pattern, path_has_glob_metachar};

    #[test]
    fn detects_glob_metachar() {
        assert!(path_has_glob_metachar("*.md"));
        assert!(!path_has_glob_metachar("readme.md"));
    }

    #[test]
    fn parent_and_glob_pattern_splits() {
        let g = parent_and_glob_pattern("/docs/*.md").unwrap();
        assert_eq!(g, Some(("/docs".into(), "*.md".into())));
        assert!(
            parent_and_glob_pattern("/docs/readme.md")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn compile_name_glob_matches() {
        let m = compile_name_glob("*.md").unwrap();
        assert!(m.is_match("a.md"));
        assert!(!m.is_match("a.txt"));
    }
}

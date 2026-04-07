//! Interactive `tb shell` — readline loop with history and completion.

use std::collections::HashMap;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::{CommandFactory, Parser};
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{Config, Context, Editor, Helper};

type TbEditor = Editor<TbHelper, DefaultHistory>;
use tabularium::resource_path::normalize_user_path;
use tabularium::rpc::Client;
use tokio::runtime::Handle;

use crate::execute::{
    ExecuteContext, ExecuteOutcome, ShellChildRpc, execute, execute_opts_from_command,
    join_abs_dir_entry,
};
use crate::shell_path::{
    resolve_ls_directory, resolve_shell_doc_path, resolve_shell_rm_path, resolve_shell_tree_scope,
};
use crate::{Command, ShellCommandOnly};

type BoxErr = Box<dyn std::error::Error + Send + Sync>;

/// Globals forwarded to nested `tb` from `tb shell` (`wait` subprocess).
pub(crate) struct ShellSpawnContext {
    pub(crate) header_flags: Vec<String>,
}

/// Clap subcommand names for `tb` (kebab-case), plus shell meta-commands (no `shell` inside shell).
/// Keep sorted alphabetically per the project coding-rules scroll.
const SUBCOMMANDS: &[&str] = &[
    "?", "append", "cat", "cd", "chat", "clear", "cp", "desc", "edit", "exit", "export", "find",
    "grep", "h", "head", "help", "import", "l", "less", "ll", "ls", "lt", "mkdir", "mv", "put",
    "q", "quit", "reindex", "rm", "search", "slice", "stat", "tail", "test", "timeout", "touch",
    "wait", "wc",
];

/// Keep sorted alphabetically per the project coding-rules scroll.
const PATH_FIRST: &[&str] = &[
    "append", "cat", "chat", "cp", "desc", "edit", "head", "less", "mv", "put", "rm", "slice",
    "stat", "tail", "touch", "wait", "wc",
];

#[cfg(test)]
mod cp_completion_tests {
    use super::{PATH_FIRST, SUBCOMMANDS};

    #[test]
    fn test_shell_cp_autocomplete_entries() {
        assert!(SUBCOMMANDS.contains(&"cp"));
        assert!(PATH_FIRST.contains(&"cp"));
    }
}

fn flags_for(sub: &str) -> &'static [&'static str] {
    match sub {
        "cat" | "slice" => &["--raw"],
        "chat" => &["-i", "--id", "--raw"],
        "cp" => &["-r", "--recursive"],
        "export" => &["-d", "--destination"],
        "find" | "search" => &["-d", "--directory"],
        "grep" => &["-m", "-v", "--invert-match"],
        "head" => &["-n", "--raw"],
        "import" => &["-n", "--name"],
        "l" | "ll" | "ls" => &["-t", "--time", "-r", "--reverse"],
        "lt" => &["-r", "--reverse"],
        "mkdir" => &["--description", "-p", "--parents"],
        "rm" => &["-r", "--recursive"],
        "tail" => &["-n", "-f", "--follow", "--raw"],
        _ => &[],
    }
}

fn is_dir_list(sub: &str) -> bool {
    sub == "ls" || sub == "l" || sub == "ll" || sub == "lt"
}

struct CompletionState {
    /// Top-level directory names under `/` (for completion when cwd is root).
    root_dir_names: Vec<String>,
    /// `list_directory` results keyed by absolute path (`"/"`, `"/notes"`, …).
    dir_list_cache: HashMap<String, Vec<(String, bool)>>,
    dirty: bool,
    /// Interactive shell only: current directory path without leading `/` (`None` = root).
    cwd: Option<String>,
}

impl Default for CompletionState {
    fn default() -> Self {
        Self {
            root_dir_names: Vec::new(),
            dir_list_cache: HashMap::new(),
            dirty: true,
            cwd: None,
        }
    }
}

impl CompletionState {
    #[cfg(test)]
    fn from_test_data(
        root_dir_names: Vec<String>,
        dirs: Vec<(&str, Vec<(&str, bool)>)>,
        cwd: Option<String>,
    ) -> Self {
        Self {
            root_dir_names,
            dir_list_cache: dirs
                .into_iter()
                .map(|(k, v)| {
                    (
                        k.to_string(),
                        v.into_iter()
                            .map(|(n, d)| (n.to_string(), d))
                            .collect::<Vec<_>>(),
                    )
                })
                .collect(),
            dirty: false,
            cwd,
        }
    }

    fn invalidate_all(&mut self) {
        self.root_dir_names.clear();
        self.dir_list_cache.clear();
        self.dirty = true;
    }

    fn ensure_root_directories(&mut self, client: &Client, handle: &Handle) {
        if !self.dirty && !self.root_dir_names.is_empty() {
            return;
        }
        if let Ok(rows) = handle.block_on(client.list_root_directories()) {
            let mut v: Vec<String> = rows.iter().map(|r| r.name().to_string()).collect();
            v.sort();
            v.dedup();
            self.root_dir_names = v;
            self.dirty = false;
        }
    }

    fn ensure_dir_list(
        &mut self,
        client: &Client,
        handle: &Handle,
        dir: &str,
    ) -> Vec<(String, bool)> {
        if let Some(d) = self.dir_list_cache.get(dir) {
            return d.clone();
        }
        let entries = match handle.block_on(client.list_directory(dir)) {
            Ok(rows) => rows
                .into_iter()
                .map(|r| (r.name().to_string(), r.is_directory()))
                .collect::<Vec<_>>(),
            Err(_) => vec![],
        };
        self.dir_list_cache.insert(dir.to_string(), entries.clone());
        entries
    }
}

fn shell_completion_parent_prefix(partial: &str, cwd: Option<&str>) -> Option<(String, String)> {
    use std::path::Path;

    let trimmed = partial.trim();
    if trimmed.is_empty() {
        let dir = cwd.map_or_else(|| "/".to_string(), |c| format!("/{c}"));
        return Some((dir, String::new()));
    }
    if trimmed == "/" {
        return Some(("/".to_string(), String::new()));
    }
    // Trailing `/` means "inside this directory" — list children (BUG H part 2).
    if trimmed.len() > 1 && trimmed.ends_with('/') {
        let merged_core = if trimmed.starts_with('/') {
            trimmed.trim_end_matches('/').to_string()
        } else if let Some(c) = cwd {
            format!(
                "/{}/{}",
                c.trim_start_matches('/').trim_end_matches('/'),
                trimmed.trim_start_matches('/').trim_end_matches('/')
            )
        } else {
            format!("/{}", trimmed.trim_start_matches('/').trim_end_matches('/'))
        };
        let merged = normalize_user_path(&merged_core).ok()?;
        return Some((merged, String::new()));
    }
    let merged_core = if trimmed.starts_with('/') {
        trimmed.trim_end_matches('/').to_string()
    } else if let Some(c) = cwd {
        format!(
            "/{}/{}",
            c.trim_start_matches('/').trim_end_matches('/'),
            trimmed.trim_start_matches('/').trim_end_matches('/')
        )
    } else {
        format!("/{}", trimmed.trim_start_matches('/').trim_end_matches('/'))
    };
    let merged = normalize_user_path(&merged_core).ok()?;
    if merged == "/" {
        return Some(("/".to_string(), String::new()));
    }
    let pb = Path::new(&merged);
    let parent = pb.parent().map_or_else(
        || "/".to_string(),
        |p| {
            let s = p.to_string_lossy();
            if s.is_empty() || s == "." {
                "/".to_string()
            } else {
                s.into_owned()
            }
        },
    );
    let parent = normalize_user_path(&parent).unwrap_or(parent);
    let prefix = pb
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    Some((parent, prefix))
}

/// `(display, replacement)` — directories show a trailing `/` in `display` only; `replacement` has
/// no trailing `/` so the user can type `/` and Tab-descend (BUG H part 1).
fn completion_path_display_and_replacement(
    full_abs: &str,
    is_dir: bool,
    abs_partial: bool,
    cwd: Option<&str>,
) -> (String, String) {
    let base = if abs_partial {
        full_abs.to_string()
    } else if let Some(c) = cwd {
        let cw = format!("/{}", c.trim_start_matches('/').trim_end_matches('/'));
        let cw_slash = format!("{cw}/");
        if full_abs == cw.as_str() {
            ".".to_string()
        } else if let Some(rest) = full_abs.strip_prefix(&cw_slash) {
            rest.to_string()
        } else {
            full_abs.trim_start_matches('/').to_string()
        }
    } else {
        full_abs.trim_start_matches('/').to_string()
    };

    if is_dir {
        (format!("{base}/"), base)
    } else {
        (base.clone(), base)
    }
}

struct TbHelper {
    client: Arc<Mutex<Client>>,
    handle: Handle,
    state: Arc<Mutex<CompletionState>>,
}

impl TbHelper {
    fn new(client: Arc<Mutex<Client>>, handle: Handle, state: Arc<Mutex<CompletionState>>) -> Self {
        Self {
            client,
            handle,
            state,
        }
    }

    fn complete_path(&self, state: &mut CompletionState, partial: &str) -> Vec<Pair> {
        self.complete_path_filtered(state, partial, false)
    }

    fn complete_path_filtered(
        &self,
        state: &mut CompletionState,
        partial: &str,
        dirs_only: bool,
    ) -> Vec<Pair> {
        let Some((parent, prefix)) = shell_completion_parent_prefix(partial, state.cwd.as_deref())
        else {
            return vec![];
        };
        let c = self
            .client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.ensure_root_directories(&c, &self.handle);
        let abs = partial.starts_with('/');
        let entries = state.ensure_dir_list(&c, &self.handle, &parent);
        let pl = prefix.to_lowercase();
        let mut pairs: Vec<Pair> = entries
            .into_iter()
            .filter(|(_, is_dir)| !dirs_only || *is_dir)
            .filter(|(name, _)| prefix.is_empty() || name.to_lowercase().starts_with(&pl))
            .map(|(name, is_dir)| {
                let full_abs = join_abs_dir_entry(&parent, &name);
                let (display, repl_core) = completion_path_display_and_replacement(
                    &full_abs,
                    is_dir,
                    abs,
                    state.cwd.as_deref(),
                );
                let (display, replacement) = if is_dir {
                    (display, repl_core)
                } else {
                    let rep = format!("{repl_core} ");
                    (repl_core.clone(), rep)
                };
                Pair {
                    display,
                    replacement,
                }
            })
            .collect();
        pairs.sort_by(|a, b| a.display.cmp(&b.display));
        pairs
    }

    // Exposed for unit tests; interactive completion uses `complete_path`.
    #[allow(dead_code)]
    fn complete_root_directories(&self, state: &mut CompletionState, partial: &str) -> Vec<Pair> {
        let c = self
            .client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.ensure_root_directories(&c, &self.handle);
        let abs = partial.starts_with('/');
        let norm = partial.strip_prefix('/').unwrap_or(partial);
        state
            .root_dir_names
            .iter()
            .filter(|c| c.starts_with(norm))
            .map(|c| {
                let rep = if abs {
                    format!("/{c} ")
                } else {
                    format!("{c} ")
                };
                Pair {
                    display: c.clone(),
                    replacement: rep,
                }
            })
            .collect()
    }

    fn complete_flags(sub: &str, partial: &str) -> Vec<Pair> {
        flags_for(sub)
            .iter()
            .filter(|f| f.starts_with(partial))
            .map(|f| Pair {
                display: (*f).to_string(),
                replacement: format!("{f} "),
            })
            .collect()
    }

    fn complete_find_shell(
        &self,
        state: &mut CompletionState,
        words: &[&str],
        ends_with_space: bool,
        partial: &str,
        start: usize,
        pos: usize,
    ) -> Option<(usize, Vec<Pair>)> {
        if partial.starts_with('-') {
            let p = Self::complete_flags("find", partial);
            return (!p.is_empty()).then_some((start, p));
        }
        if words.len() == 1 && ends_with_space {
            return Some((
                pos,
                vec![
                    Pair {
                        display: "-d".into(),
                        replacement: "-d ".into(),
                    },
                    Pair {
                        display: "--directory".into(),
                        replacement: "--directory ".into(),
                    },
                ],
            ));
        }

        let d_pos = words.iter().position(|&w| w == "-d" || w == "--directory");
        if let Some(ci) = d_pos {
            let dir_idx = ci + 1;
            if dir_idx >= words.len() && ends_with_space {
                let mut p = vec![Pair {
                    display: "/".into(),
                    replacement: "/ ".into(),
                }];
                p.extend(self.complete_path(state, ""));
                return Some((pos, p));
            }
            if dir_idx < words.len() {
                if ends_with_space && words.len() == dir_idx + 1 {
                    return Some((pos, vec![]));
                }
                if !ends_with_space && words.len() == dir_idx + 1 {
                    let pref = words[dir_idx];
                    if !pref.starts_with('-') {
                        let mut p = vec![];
                        if "/".starts_with(pref) {
                            p.push(Pair {
                                display: "/".into(),
                                replacement: "/ ".into(),
                            });
                        }
                        p.extend(self.complete_path(state, pref));
                        return Some((start, p));
                    }
                }
            }
        }

        None
    }

    fn complete_grep(
        &self,
        state: &mut CompletionState,
        words: &[&str],
        ends_with_space: bool,
        partial: &str,
        start: usize,
        pos: usize,
    ) -> Option<(usize, Vec<Pair>)> {
        if partial.starts_with('-') {
            let p = Self::complete_flags("grep", partial);
            return (!p.is_empty()).then_some((start, p));
        }
        if words.len() < 2 {
            return None;
        }
        let mut i = 1usize;
        while i < words.len() {
            match words[i] {
                "-v" | "--invert-match" => i += 1,
                "-m" | "--max-matches" => {
                    i += 1;
                    if i < words.len() && !words[i].starts_with('-') {
                        i += 1;
                    }
                }
                w if w.starts_with('-') => i += 1,
                _ => break,
            }
        }
        let rest = &words[i..];
        if ends_with_space && words.len() >= 2 {
            let last = words[words.len() - 1];
            if last == "-m" || last == "--max-matches" {
                return Some((pos, vec![]));
            }
            if matches!(last, "-v" | "--invert-match") {
                return Some((pos, vec![]));
            }
        }
        match rest.len() {
            0 => {
                if ends_with_space && words.len() > 1 {
                    let last = words[words.len() - 1];
                    if !last.starts_with('-') && last != words[0] {
                        let p = self.complete_path(state, "");
                        return Some((pos, p));
                    }
                }
                None
            }
            1 => {
                if ends_with_space {
                    let p = self.complete_path(state, "");
                    Some((pos, p))
                } else {
                    Some((start, vec![]))
                }
            }
            n if n >= 2 => {
                if ends_with_space {
                    None
                } else {
                    let path_arg = *rest.last().unwrap();
                    let p = self.complete_path(state, path_arg);
                    Some((start, p))
                }
            }
            _ => None,
        }
    }

    fn complete_search(
        &self,
        state: &mut CompletionState,
        words: &[&str],
        ends_with_space: bool,
        partial: &str,
        start: usize,
        pos: usize,
    ) -> Option<(usize, Vec<Pair>)> {
        if partial.starts_with('-') {
            let p = Self::complete_flags("search", partial);
            return (!p.is_empty()).then_some((start, p));
        }
        if words.len() < 2 {
            return None;
        }
        let d_pos = words.iter().position(|&w| w == "-d" || w == "--directory");
        if let Some(ci) = d_pos {
            let di = ci + 1;
            if di >= words.len() {
                if ends_with_space {
                    let mut p = vec![Pair {
                        display: "/".into(),
                        replacement: "/ ".into(),
                    }];
                    p.extend(self.complete_path(state, ""));
                    return Some((pos, p));
                }
            } else if di + 1 == words.len() && !ends_with_space {
                let pref = words[di];
                let mut p = vec![];
                if "/".starts_with(pref) {
                    p.push(Pair {
                        display: "/".into(),
                        replacement: "/ ".into(),
                    });
                }
                p.extend(self.complete_path(state, pref));
                return Some((start, p));
            }
        }
        None
    }

    fn complete_ls_shell(
        &self,
        state: &mut CompletionState,
        words: &[&str],
        ends_with_space: bool,
        partial: &str,
        start: usize,
        pos: usize,
    ) -> Option<(usize, Vec<Pair>)> {
        if words.len() < 2 {
            return None;
        }
        if partial.starts_with('-') {
            let p = Self::complete_flags(words[0], partial);
            return (!p.is_empty()).then_some((start, p));
        }
        let mut i = 1usize;
        while i < words.len() {
            match words[i] {
                "-t" | "--time" | "-r" | "--reverse" => i += 1,
                w if w.starts_with('-') => i += 1,
                _ => break,
            }
        }
        let rest = &words[i..];
        match rest.len() {
            0 => {
                if ends_with_space {
                    let mut p = vec![Pair {
                        display: "/".into(),
                        replacement: "/ ".into(),
                    }];
                    p.extend(self.complete_path(state, ""));
                    Some((pos, p))
                } else {
                    None
                }
            }
            1 => {
                if ends_with_space {
                    None
                } else {
                    let w = rest[0];
                    let mut p = vec![];
                    if "/".starts_with(w) {
                        p.push(Pair {
                            display: "/".into(),
                            replacement: "/ ".into(),
                        });
                    }
                    p.extend(self.complete_path(state, w));
                    Some((start, p))
                }
            }
            _ => None,
        }
    }
}

/// Files and directories under `partial`'s parent (for `import` local source).
fn complete_import_fs_entries(partial: &str) -> Vec<Pair> {
    let (base_dir, prefix) = resolve_dir_prefix(partial);
    let Ok(rd) = std::fs::read_dir(&base_dir) else {
        return vec![];
    };
    let mut out = Vec::new();
    for ent in rd.flatten() {
        let path = ent.path();
        let name = ent.file_name().to_string_lossy().to_string();
        if !prefix.is_empty() && !name.starts_with(&prefix) {
            continue;
        }
        let joined = base_dir.join(&name);
        let display_path = joined.display().to_string();
        if path.is_dir() {
            out.push(Pair {
                display: format!("{display_path}/"),
                replacement: format!("{display_path} "),
            });
        } else {
            out.push(Pair {
                display: display_path.clone(),
                replacement: format!("{display_path} "),
            });
        }
    }
    out.sort_by(|a, b| a.display.cmp(&b.display));
    out
}

/// Directory entries for path completion (`export -d` destination).
fn complete_dir_entries(partial: &str) -> Vec<Pair> {
    let (base_dir, prefix) = resolve_dir_prefix(partial);
    let Ok(rd) = std::fs::read_dir(&base_dir) else {
        return vec![];
    };
    let mut out = Vec::new();
    for ent in rd.flatten() {
        if !ent.path().is_dir() {
            continue;
        }
        let name = ent.file_name().to_string_lossy().to_string();
        if !prefix.is_empty() && !name.starts_with(&prefix) {
            continue;
        }
        let joined = base_dir.join(&name);
        let display = joined.display().to_string();
        out.push(Pair {
            display: format!("{display}/"),
            replacement: format!("{display} "),
        });
    }
    out.sort_by(|a, b| a.display.cmp(&b.display));
    out
}

fn resolve_dir_prefix(partial: &str) -> (PathBuf, String) {
    if partial.is_empty() {
        let dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        return (dir, String::new());
    }
    let p = Path::new(partial);
    if partial.ends_with('/') || partial.ends_with('\\') {
        return (p.to_path_buf(), String::new());
    }
    match p.parent() {
        Some(pa) if pa.as_os_str().is_empty() => (
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            p.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string(),
        ),
        Some(pa) => (
            pa.to_path_buf(),
            p.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string(),
        ),
        None => (
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            partial.to_string(),
        ),
    }
}

impl Completer for TbHelper {
    type Candidate = Pair;

    #[allow(clippy::too_many_lines)]
    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let start = word_start(line, pos);
        let partial = &line[start..pos];
        let before = &line[..pos];
        let words: Vec<&str> = before.split_whitespace().collect();
        let ends_with_space = line[..pos].chars().last().is_some_and(char::is_whitespace);

        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // First token: subcommands
        if words.is_empty() || (words.len() == 1 && !ends_with_space) {
            let p: Vec<Pair> = SUBCOMMANDS
                .iter()
                .filter(|s| s.starts_with(partial))
                .map(|s| Pair {
                    display: (*s).to_string(),
                    replacement: format!("{s} "),
                })
                .collect();
            return Ok((start, p));
        }

        let sub = words[0];

        if sub == "timeout" && words.len() > 1 {
            return Ok((start, vec![]));
        }

        // After subcommand + space: optional root dir for ls, else path or flags
        if words.len() == 1 && ends_with_space {
            if is_dir_list(sub) {
                let mut p = vec![Pair {
                    display: "/".into(),
                    replacement: "/ ".into(),
                }];
                p.extend(self.complete_path(&mut state, ""));
                p.extend(TbHelper::complete_flags(sub, ""));
                return Ok((pos, p));
            }
            if sub == "cd" {
                let mut p = vec![Pair {
                    display: "/".into(),
                    replacement: "/ ".into(),
                }];
                p.extend(self.complete_path_filtered(&mut state, "", true));
                return Ok((pos, p));
            }
            if sub == "import" {
                let mut p = vec![Pair {
                    display: "/".into(),
                    replacement: "/ ".into(),
                }];
                p.extend(self.complete_path(&mut state, ""));
                return Ok((pos, p));
            }
            if sub == "export" {
                let mut p = vec![Pair {
                    display: "/".into(),
                    replacement: "/ ".into(),
                }];
                p.extend(self.complete_path(&mut state, ""));
                return Ok((pos, p));
            }
            if PATH_FIRST.contains(&sub) {
                let p = self.complete_path(&mut state, "");
                return Ok((pos, p));
            }
            if sub == "mkdir" {
                let p = self.complete_path(&mut state, "");
                return Ok((pos, p));
            }
            if sub == "reindex" {
                let mut p = vec![Pair {
                    display: "/".into(),
                    replacement: "/ ".into(),
                }];
                p.extend(self.complete_path(&mut state, ""));
                return Ok((pos, p));
            }
            let fl = TbHelper::complete_flags(sub, "");
            if !fl.is_empty() {
                return Ok((pos, fl));
            }
            return Ok((start, vec![]));
        }

        if is_dir_list(sub)
            && let Some(r) =
                self.complete_ls_shell(&mut state, &words, ends_with_space, partial, start, pos)
        {
            return Ok(r);
        }

        // `cd <partial>`
        if sub == "cd" && words.len() == 2 && !ends_with_space {
            let w = words[1];
            let mut p = vec![];
            if "/".starts_with(w) {
                p.push(Pair {
                    display: "/".into(),
                    replacement: "/ ".into(),
                });
            }
            p.extend(self.complete_path_filtered(&mut state, w, true));
            return Ok((start, p));
        }

        if sub == "find"
            && let Some(r) =
                self.complete_find_shell(&mut state, &words, ends_with_space, partial, start, pos)
        {
            return Ok(r);
        }

        if sub == "import" {
            if partial.starts_with('-') {
                let p = TbHelper::complete_flags("import", partial);
                if !p.is_empty() {
                    return Ok((start, p));
                }
            }
            if words.len() == 2 && !ends_with_space {
                let w = words[1];
                let mut pairs = Vec::new();
                if "/".starts_with(w) {
                    pairs.push(Pair {
                        display: "/".into(),
                        replacement: "/ ".into(),
                    });
                }
                pairs.extend(self.complete_path(&mut state, w));
                return Ok((start, pairs));
            }
            if words.len() == 2 && ends_with_space {
                return Ok((pos, complete_import_fs_entries("")));
            }
            if words.len() == 3 && !ends_with_space {
                return Ok((start, complete_import_fs_entries(words[2])));
            }
        }

        if sub == "export" {
            if words.len() == 2 && !ends_with_space {
                let w = words[1];
                let mut pairs = Vec::new();
                if "/".starts_with(w) {
                    pairs.push(Pair {
                        display: "/".into(),
                        replacement: "/ ".into(),
                    });
                }
                pairs.extend(self.complete_path(&mut state, w));
                return Ok((start, pairs));
            }
            if words.len() == 2 && ends_with_space {
                let fl = TbHelper::complete_flags(sub, "");
                if !fl.is_empty() {
                    return Ok((pos, fl));
                }
                return Ok((start, vec![]));
            }
            if ends_with_space && matches!(words.last().copied(), Some("-d" | "--destination")) {
                return Ok((pos, complete_dir_entries("")));
            }
            if words.len() >= 3 {
                let prev = words[words.len() - 2];
                let cur = words[words.len() - 1];
                if matches!(prev, "-d" | "--destination")
                    && !cur.starts_with('-')
                    && !ends_with_space
                {
                    return Ok((start, complete_dir_entries(cur)));
                }
            }
        }

        if sub == "grep"
            && let Some(r) =
                self.complete_grep(&mut state, &words, ends_with_space, partial, start, pos)
        {
            return Ok(r);
        }

        if sub == "search"
            && let Some(r) =
                self.complete_search(&mut state, &words, ends_with_space, partial, start, pos)
        {
            return Ok(r);
        }

        if sub == "reindex" && words.len() == 2 && !ends_with_space {
            let w = words[1];
            let mut pairs = Vec::new();
            if "/".starts_with(w) {
                pairs.push(Pair {
                    display: "/".into(),
                    replacement: "/ ".into(),
                });
            }
            pairs.extend(self.complete_path(&mut state, w));
            return Ok((start, pairs));
        }

        // Flag completion
        if partial.starts_with('-') && !words.is_empty() {
            let p = TbHelper::complete_flags(sub, partial);
            if !p.is_empty() {
                return Ok((start, p));
            }
        }

        // `mv src <Tab>` — complete destination (empty partial), not re-filter `src`.
        if sub == "mv" && words.len() == 2 && ends_with_space {
            let p = self.complete_path(&mut state, "");
            return Ok((pos, p));
        }

        // Path-like second (or later) token for path-first commands
        if PATH_FIRST.contains(&sub) && words.len() >= 2 {
            let arg = words[words.len() - 1];
            if !arg.starts_with('-') {
                let p = self.complete_path(&mut state, arg);
                return Ok((start, p));
            }
        }

        if sub == "rm" {
            let has_rec = words.iter().any(|&w| w == "-r" || w == "--recursive");
            if has_rec {
                if ends_with_space && matches!(words.last().copied(), Some("-r" | "--recursive")) {
                    let p = self.complete_path(&mut state, "");
                    return Ok((pos, p));
                }
                if !ends_with_space && words.len() >= 2 {
                    let last = words[words.len() - 1];
                    if !last.starts_with('-') {
                        let p = self.complete_path(&mut state, last);
                        return Ok((start, p));
                    }
                    if words.len() == 2 {
                        return Ok((start, vec![]));
                    }
                }
            }
        }

        if sub == "mkdir" && words.len() == 2 && !ends_with_space && !words[1].starts_with('-') {
            let p = self.complete_path(&mut state, words[1]);
            return Ok((start, p));
        }
        if sub == "mkdir"
            && words.len() == 3
            && !ends_with_space
            && (words[1] == "-p" || words[1] == "--parents")
        {
            let p = self.complete_path(&mut state, words[2]);
            return Ok((start, p));
        }

        Ok((start, vec![]))
    }
}

impl Hinter for TbHelper {
    type Hint = String;
}

impl Highlighter for TbHelper {}

impl Validator for TbHelper {}

impl Helper for TbHelper {}

fn word_start(line: &str, pos: usize) -> usize {
    line[..pos]
        .char_indices()
        .filter(|(_, c)| c.is_whitespace())
        .next_back()
        .map_or(0, |(i, c)| i + c.len_utf8())
}

fn has_explicit_port(authority: &str) -> bool {
    if let Some(inner) = authority.strip_prefix('[') {
        if let Some(idx) = inner.find(']') {
            let after = &inner[idx + 1..];
            return after.starts_with(':')
                && !after[1..].is_empty()
                && after[1..].chars().all(|c| c.is_ascii_digit());
        }
        false
    } else if let Some(colon) = authority.rfind(':') {
        let tail = &authority[colon + 1..];
        !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit())
    } else {
        false
    }
}

fn prompt_target(api_uri: &str) -> String {
    let trimmed = api_uri.trim();
    let rest = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
        .unwrap_or(trimmed);
    let authority = rest.split('/').next().unwrap_or(rest);
    if authority.is_empty() {
        return "?".into();
    }
    if has_explicit_port(authority) {
        authority.to_string()
    } else if let Some(stripped) = authority.strip_prefix('[') {
        if let Some(end) = stripped.find(']') {
            format!("[{}]", &stripped[..end])
        } else {
            authority.to_string()
        }
    } else {
        authority
            .split_once(':')
            .map_or_else(|| authority.to_string(), |(h, _)| h.to_string())
    }
}

fn prompt_tuple(api_uri: &str, cwd: Option<&str>) -> (String, String) {
    let target = prompt_target(api_uri);
    let scope = match cwd {
        None => format!("{target}/"),
        Some(c) => format!("{target}/{c}"),
    };
    let raw = format!("tb {scope} ❯ ");
    let styled = format!("\x1b[1;34mtb\x1b[0m \x1b[36m{scope}\x1b[0m \x1b[1;33m❯\x1b[0m ");
    let no_color = std::env::var_os("NO_COLOR").is_some();
    if no_color || !io::stdout().is_terminal() {
        (raw.clone(), raw)
    } else {
        (raw, styled)
    }
}

fn should_invalidate_cache(cmd: &Command) -> bool {
    matches!(
        cmd,
        Command::Put { .. }
            | Command::Import { .. }
            | Command::Append { .. }
            | Command::Chat { .. }
            | Command::Rm { .. }
            | Command::Mv { .. }
            | Command::Cp { .. }
            | Command::Mkdir { .. }
            | Command::Touch { .. }
            | Command::Desc { .. }
    )
}

/// Interactive shell only: resolve path segments against `shell_cwd` before RPC.
#[allow(clippy::too_many_lines)]
pub(crate) fn apply_shell_cwd(cmd: Command, shell_cwd: Option<&str>) -> Result<Command, String> {
    Ok(match cmd {
        Command::Ls {
            directory,
            sort_by_time,
            reverse,
        } => Command::Ls {
            directory: resolve_ls_directory(directory.as_deref(), shell_cwd)?,
            sort_by_time,
            reverse,
        },
        Command::Lt { directory, reverse } => Command::Lt {
            directory: resolve_ls_directory(directory.as_deref(), shell_cwd)?,
            reverse,
        },
        Command::Append { path, file } => Command::Append {
            path: resolve_shell_doc_path(&path, shell_cwd)?,
            file,
        },
        Command::Cat { path, raw } => Command::Cat {
            path: resolve_shell_doc_path(&path, shell_cwd)?,
            raw,
        },
        Command::Desc { path, description } => Command::Desc {
            path: resolve_shell_doc_path(&path, shell_cwd)?,
            description,
        },
        Command::Put { path, file } => Command::Put {
            path: resolve_shell_doc_path(&path, shell_cwd)?,
            file,
        },
        Command::Rm { path, recursive } => Command::Rm {
            path: resolve_shell_rm_path(&path, recursive, shell_cwd)?,
            recursive,
        },
        Command::Head { path, lines, raw } => Command::Head {
            path: resolve_shell_doc_path(&path, shell_cwd)?,
            lines,
            raw,
        },
        Command::Tail {
            path,
            tail,
            follow,
            raw,
        } => Command::Tail {
            path: resolve_shell_doc_path(&path, shell_cwd)?,
            tail,
            follow,
            raw,
        },
        Command::Chat {
            path,
            nickname,
            raw,
        } => Command::Chat {
            path: resolve_shell_doc_path(&path, shell_cwd)?,
            nickname,
            raw,
        },
        Command::Slice {
            path,
            start,
            end,
            raw,
        } => Command::Slice {
            path: resolve_shell_doc_path(&path, shell_cwd)?,
            start,
            end,
            raw,
        },
        Command::Grep {
            pattern,
            path,
            max_matches,
            invert_match,
        } => Command::Grep {
            pattern,
            path: resolve_shell_doc_path(&path, shell_cwd)?,
            max_matches,
            invert_match,
        },
        Command::Stat { path } => Command::Stat {
            path: resolve_shell_doc_path(&path, shell_cwd)?,
        },
        Command::Wc { path } => Command::Wc {
            path: resolve_shell_doc_path(&path, shell_cwd)?,
        },
        Command::Edit { path } => Command::Edit {
            path: resolve_shell_doc_path(&path, shell_cwd)?,
        },
        Command::Wait { path } => Command::Wait {
            path: resolve_shell_doc_path(&path, shell_cwd)?,
        },
        Command::Mv { src, dst } => Command::Mv {
            src: resolve_shell_doc_path(&src, shell_cwd)?,
            dst: resolve_shell_doc_path(&dst, shell_cwd)?,
        },
        Command::Cp {
            recursive,
            src,
            dst,
        } => Command::Cp {
            recursive,
            src: resolve_shell_tree_scope(&src, shell_cwd)?,
            dst: resolve_shell_tree_scope(&dst, shell_cwd)?,
        },
        Command::Search { directory, query } => Command::Search {
            directory: match directory {
                None => None,
                Some(c) => Some(resolve_shell_tree_scope(&c, shell_cwd)?),
            },
            query,
        },
        Command::Find { directory, name } => Command::Find {
            directory: match directory {
                None => None,
                Some(c) => Some(resolve_shell_tree_scope(&c, shell_cwd)?),
            },
            name,
        },
        Command::Import {
            directory,
            dir,
            name,
        } => Command::Import {
            directory: resolve_shell_tree_scope(&directory, shell_cwd)?,
            dir,
            name,
        },
        Command::Less { path } => Command::Less {
            path: resolve_shell_doc_path(&path, shell_cwd)?,
        },
        Command::Export {
            directory,
            destination,
        } => Command::Export {
            directory: resolve_shell_tree_scope(&directory, shell_cwd)?,
            destination,
        },
        Command::Mkdir {
            path,
            description,
            parents,
        } => Command::Mkdir {
            path: resolve_shell_doc_path(&path, shell_cwd)?,
            description,
            parents,
        },
        Command::Touch { path, time } => Command::Touch {
            path: resolve_shell_doc_path(&path, shell_cwd)?,
            time,
        },
        Command::Reindex { path } => Command::Reindex {
            path: resolve_shell_tree_scope(&path, shell_cwd)?,
        },
        Command::Test => Command::Test,
        Command::Shell { cwd } => Command::Shell {
            cwd: match cwd {
                None => None,
                Some(p) => {
                    let t = p.trim();
                    if t.is_empty() {
                        None
                    } else {
                        let r = resolve_shell_tree_scope(t, shell_cwd)?;
                        if r == "/" { None } else { Some(r) }
                    }
                }
            },
        },
    })
}

/// Resolves a `cd` target to a canonical absolute path for `list_directory`.
pub(crate) fn merge_cd_path(target: &str, cwd: Option<&str>) -> Result<String, String> {
    let target = target.trim();
    if target.is_empty() {
        return Err("cd: empty path".into());
    }
    let s = resolve_shell_tree_scope(target, cwd).map_err(|e| format!("cd: {e}"))?;
    Ok(if s == "/" {
        "/".to_string()
    } else {
        format!("/{s}")
    })
}

fn handle_shell_cd(
    client: &Arc<Mutex<Client>>,
    handle: &Handle,
    state: &Arc<Mutex<CompletionState>>,
    trimmed: &str,
) -> Result<(), String> {
    let words: Vec<String> =
        shell_words::split(trimmed).map_err(|_| "cd: unmatched quote".to_string())?;
    if words.first().map(String::as_str) != Some("cd") {
        return Err("internal: expected cd".into());
    }
    if words.len() == 1 {
        return Err("cd: missing operand".into());
    }
    if words.len() > 2 {
        return Err("cd: too many arguments".into());
    }
    let target = words[1].as_str();
    let mut g = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match target.trim() {
        "/" => {
            g.cwd = None;
            g.invalidate_all();
            Ok(())
        }
        ".." => {
            g.cwd = g.cwd.as_ref().and_then(|c| {
                c.rsplit_once('/')
                    .and_then(|(p, _)| (!p.is_empty()).then(|| p.to_string()))
            });
            g.invalidate_all();
            Ok(())
        }
        t => {
            let path = merge_cd_path(t, g.cwd.as_deref())?;
            let c = client
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            handle
                .block_on(c.list_directory(&path))
                .map_err(|e| format!("cd: {e} (not a directory or inaccessible)"))?;
            if path == "/" {
                g.cwd = None;
            } else {
                g.cwd = Some(path.trim_start_matches('/').to_string());
            }
            g.invalidate_all();
            Ok(())
        }
    }
}

fn handle_shell_timeout(
    words: &[String],
    client: &Arc<Mutex<Client>>,
    shell_child_rpc: &mut ShellChildRpc,
) -> Result<(), String> {
    if words.first().map(String::as_str) != Some("timeout") {
        return Err("internal: expected timeout".into());
    }
    match words.len() {
        1 => {
            println!("{}s", shell_child_rpc.timeout_sec());
            Ok(())
        }
        2 => {
            let n: u64 = words[1]
                .parse()
                .map_err(|_| "timeout: invalid number".to_string())?;
            let n = n.max(1);
            let new_c = {
                let c = client
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                c.with_timeout(Duration::from_secs(n))
                    .map_err(|e| e.to_string())?
            };
            *client
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = new_c;
            shell_child_rpc.set_timeout_sec(n);
            println!("timeout set to {n}s");
            Ok(())
        }
        _ => Err("timeout: too many arguments".into()),
    }
}

fn push_history(
    rl: &mut TbEditor,
    history_path: &PathBuf,
    line: &str,
) -> Result<(), ReadlineError> {
    if line.is_empty() {
        return Ok(());
    }
    rl.add_history_entry(line)?;
    rl.append_history(history_path)?;
    Ok(())
}

fn install_initial_shell_cwd(
    client: &Arc<Mutex<Client>>,
    handle: &Handle,
    state: &Arc<Mutex<CompletionState>>,
    cwd: Option<String>,
) -> Result<(), BoxErr> {
    let Some(s) = cwd else {
        return Ok(());
    };
    let t = s.trim();
    if t.is_empty() {
        return Ok(());
    }
    let path = merge_cd_path(t, None).map_err(|e| -> BoxErr {
        if let Some(stripped) = e.strip_prefix("cd: ") {
            format!("shell: {stripped}").into()
        } else {
            e.into()
        }
    })?;
    let c = client
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    handle
        .block_on(c.list_directory(&path))
        .map_err(|e| -> BoxErr { format!("shell: not a directory or inaccessible: {e}").into() })?;
    let mut g = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    g.cwd = if path == "/" {
        None
    } else {
        Some(path.trim_start_matches('/').to_string())
    };
    g.invalidate_all();
    Ok(())
}

fn print_shell_help() -> Result<(), BoxErr> {
    let mut cmd = ShellCommandOnly::command();
    cmd.build();
    cmd.print_long_help()?;
    Ok(())
}

fn clear_screen() -> io::Result<()> {
    if io::stdout().is_terminal() {
        print!("\x1b[2J\x1b[H");
        io::stdout().flush()?;
    }
    Ok(())
}

/// Run a line in the user's login shell (`SHELL`), or `/bin/sh` on Unix / `COMSPEC` on Windows when unset.
fn exec_shell_line(script: &str) -> io::Result<std::process::ExitStatus> {
    #[cfg(unix)]
    let _sig_guard = crate::execute::SigIntIgnoreWhileWaiting::new()
        .map_err(|e| io::Error::other(e.to_string()))?;

    #[cfg(unix)]
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    #[cfg(not(unix))]
    let shell = std::env::var("SHELL")
        .unwrap_or_else(|_| std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string()));

    let mut cmd = StdCommand::new(&shell);
    #[cfg(not(unix))]
    {
        let lower = shell.to_lowercase();
        if lower.ends_with("cmd.exe") || lower == "cmd" {
            cmd.arg("/C");
        } else {
            cmd.arg("-c");
        }
    }
    #[cfg(unix)]
    cmd.arg("-c");
    cmd.arg(script);
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            Ok(())
        });
    }
    cmd.status()
}

fn build_editor(
    client: Arc<Mutex<Client>>,
    handle: Handle,
    state: Arc<Mutex<CompletionState>>,
    history_path: &PathBuf,
) -> Result<TbEditor, ReadlineError> {
    let config = Config::builder().auto_add_history(false).build();
    let mut rl = Editor::with_config(config)?;
    let helper = TbHelper::new(client, handle, state);
    rl.set_helper(Some(helper));
    if history_path.exists() {
        rl.load_history(history_path)?;
    }
    Ok(rl)
}

#[allow(clippy::too_many_lines)]
fn run_shell_blocking(
    client: Arc<Mutex<Client>>,
    handle: Handle,
    history_path: PathBuf,
    api_uri: String,
    timeout_sec: u64,
    initial_cwd: Option<String>,
    spawn_ctx: ShellSpawnContext,
) -> Result<(), BoxErr> {
    let state = Arc::new(Mutex::new(CompletionState::default()));
    install_initial_shell_cwd(&client, &handle, &state, initial_cwd)?;
    let timeout_sec = timeout_sec.max(1);
    let mut shell_child_rpc =
        ShellChildRpc::new(api_uri.clone(), timeout_sec, spawn_ctx.header_flags);
    let mut rl = build_editor(
        Arc::clone(&client),
        handle.clone(),
        Arc::clone(&state),
        &history_path,
    )?;

    loop {
        let prompt_line = {
            let g = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let (raw, styled) = prompt_tuple(&api_uri, g.cwd.as_deref());
            let use_style = std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal();
            if use_style { styled } else { raw }
        };
        let readline = rl.readline(&prompt_line);
        match readline {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                match trimmed {
                    "exit" | "quit" | "q" => {
                        push_history(&mut rl, &history_path, trimmed)?;
                        rl.save_history(&history_path)?;
                        break;
                    }
                    "help" | "h" | "?" => {
                        push_history(&mut rl, &history_path, trimmed)?;
                        if let Err(e) = print_shell_help() {
                            eprintln!("{e}");
                        }
                        continue;
                    }
                    "clear" => {
                        push_history(&mut rl, &history_path, trimmed)?;
                        let _ = clear_screen();
                        continue;
                    }
                    _ => {}
                }

                if trimmed.split_whitespace().next() == Some("cd") {
                    match handle_shell_cd(&client, &handle, &state, trimmed) {
                        Ok(()) => {
                            push_history(&mut rl, &history_path, trimmed)?;
                        }
                        Err(e) => {
                            eprintln!("{e}");
                            push_history(&mut rl, &history_path, trimmed)?;
                        }
                    }
                    continue;
                }

                if trimmed.split_whitespace().next() == Some("timeout") {
                    let Ok(words) = shell_words::split(trimmed) else {
                        eprintln!("missing closing quote");
                        push_history(&mut rl, &history_path, trimmed)?;
                        continue;
                    };
                    if words.first().map(String::as_str) == Some("timeout") {
                        match handle_shell_timeout(&words, &client, &mut shell_child_rpc) {
                            Ok(()) => {
                                push_history(&mut rl, &history_path, trimmed)?;
                            }
                            Err(e) => {
                                eprintln!("{e}");
                                push_history(&mut rl, &history_path, trimmed)?;
                            }
                        }
                    }
                    continue;
                }

                if let Some(rest) = trimmed.strip_prefix('!') {
                    let script = rest.trim_start();
                    if script.is_empty() {
                        eprintln!("empty shell command after !");
                        push_history(&mut rl, &history_path, trimmed)?;
                        continue;
                    }
                    if let Err(e) = exec_shell_line(script) {
                        eprintln!("{e}");
                    }
                    push_history(&mut rl, &history_path, trimmed)?;
                    continue;
                }

                let Ok(words) = shell_words::split(trimmed) else {
                    eprintln!("missing closing quote");
                    push_history(&mut rl, &history_path, trimmed)?;
                    continue;
                };

                let parsed = match ShellCommandOnly::try_parse_from(
                    std::iter::once("tb".to_string()).chain(words),
                ) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("{e}");
                        push_history(&mut rl, &history_path, trimmed)?;
                        continue;
                    }
                };

                let cmd = {
                    let g = state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    match apply_shell_cwd(parsed.command, g.cwd.as_deref()) {
                        Ok(c) => c,
                        Err(e) => {
                            eprintln!("{e}");
                            push_history(&mut rl, &history_path, trimmed)?;
                            continue;
                        }
                    }
                };

                if matches!(cmd, Command::Edit { .. }) {
                    push_history(&mut rl, &history_path, trimmed)?;
                    drop(rl);
                    let exec_opts = execute_opts_from_command(&cmd);
                    let exec_client = client
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    match handle.block_on(execute(
                        &exec_client,
                        cmd.clone(),
                        ExecuteContext::Shell,
                        Some(&shell_child_rpc),
                        exec_opts,
                    )) {
                        Ok(ExecuteOutcome::Ok | ExecuteOutcome::Interrupted) => {
                            if should_invalidate_cache(&cmd) {
                                let mut g = state
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                g.invalidate_all();
                            }
                        }
                        Err(e) => eprintln!("{e}"),
                    }
                    rl = build_editor(
                        Arc::clone(&client),
                        handle.clone(),
                        Arc::clone(&state),
                        &history_path,
                    )?;
                    continue;
                }

                let exec_opts = execute_opts_from_command(&cmd);
                let exec_client = client
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match handle.block_on(execute(
                    &exec_client,
                    cmd.clone(),
                    ExecuteContext::Shell,
                    Some(&shell_child_rpc),
                    exec_opts,
                )) {
                    Ok(ExecuteOutcome::Ok | ExecuteOutcome::Interrupted) => {
                        if should_invalidate_cache(&cmd) {
                            let mut g = state
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            g.invalidate_all();
                        }
                        push_history(&mut rl, &history_path, trimmed)?;
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        push_history(&mut rl, &history_path, trimmed)?;
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C");
            }
            Err(ReadlineError::Eof) => {
                let _ = rl.save_history(&history_path);
                break;
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

pub(crate) async fn run_shell(
    client: Client,
    api_uri: String,
    timeout_sec: u64,
    initial_cwd: Option<String>,
    spawn_ctx: ShellSpawnContext,
) -> Result<(), BoxErr> {
    if !io::stdin().is_terminal() {
        return Err("interactive shell requires a terminal".into());
    }
    let home = homedir::my_home()
        .map_err(|e| format!("home directory: {e}"))?
        .ok_or_else(|| "could not resolve home directory".to_string())?;
    let history_path = home.join(".tb_history");
    let client = Arc::new(Mutex::new(client));
    let handle = Handle::current();
    tokio::task::spawn_blocking(move || {
        run_shell_blocking(
            client,
            handle,
            history_path,
            api_uri,
            timeout_sec,
            initial_cwd,
            spawn_ctx,
        )
    })
    .await??;
    Ok(())
}

#[cfg(test)]
mod shell_cwd_tests {
    use crate::Command;
    use tabularium::TailMode;

    #[allow(clippy::similar_names)]
    fn unwrap_ac(cmd: Command, cwd: Option<&str>) -> Command {
        super::apply_shell_cwd(cmd, cwd).unwrap()
    }

    #[test]
    fn ls_fills_cwd_when_omitted() {
        let c = unwrap_ac(
            Command::Ls {
                directory: None,
                sort_by_time: false,
                reverse: false,
            },
            Some("notes"),
        );
        assert!(matches!(c, Command::Ls { directory: Some(ref s), .. } if s == "notes"));
    }

    #[test]
    fn ls_bare_segment_prepends_cwd_like_cat() {
        let c = unwrap_ac(
            Command::Ls {
                directory: Some("other".into()),
                sort_by_time: false,
                reverse: false,
            },
            Some("notes"),
        );
        assert!(matches!(c, Command::Ls { directory: Some(ref s), .. } if s == "notes/other"));
    }

    #[test]
    fn ls_dot_resolves_shell_cwd() {
        let c = unwrap_ac(
            Command::Ls {
                directory: Some(".".into()),
                sort_by_time: false,
                reverse: false,
            },
            Some("notes"),
        );
        assert!(matches!(c, Command::Ls { directory: Some(ref s), .. } if s == "notes"));
    }

    #[test]
    fn cat_bare_prepends_slash_bypasses() {
        let c = unwrap_ac(
            Command::Cat {
                path: "doc".into(),
                raw: false,
            },
            Some("n"),
        );
        assert!(matches!(c, Command::Cat { path, .. } if path == "n/doc"));
        let c = unwrap_ac(
            Command::Cat {
                path: "x/doc".into(),
                raw: false,
            },
            Some("n"),
        );
        assert!(matches!(c, Command::Cat { path, .. } if path == "n/x/doc"));
    }

    #[test]
    fn rm_multi_segment_uses_cwd() {
        let c = unwrap_ac(
            Command::Rm {
                path: "chats/chat1".into(),
                recursive: false,
            },
            Some("xxx"),
        );
        assert!(matches!(c, Command::Rm { path, .. } if path == "xxx/chats/chat1"));
    }

    #[test]
    fn mv_src_multi_segment_uses_cwd() {
        let c = unwrap_ac(
            Command::Mv {
                src: "subdir/file".into(),
                dst: "target".into(),
            },
            Some("xxx"),
        );
        assert!(
            matches!(c, Command::Mv { src, dst } if src == "xxx/subdir/file" && dst == "xxx/target")
        );
    }

    #[test]
    fn cat_multi_segment_uses_cwd() {
        let c = unwrap_ac(
            Command::Cat {
                path: "subdir/doc".into(),
                raw: false,
            },
            Some("xxx"),
        );
        assert!(matches!(c, Command::Cat { path, .. } if path == "xxx/subdir/doc"));
    }

    #[test]
    fn put_multi_segment_uses_cwd() {
        let c = unwrap_ac(
            Command::Put {
                path: "subdir/doc".into(),
                file: None,
            },
            Some("xxx"),
        );
        assert!(matches!(c, Command::Put { path, .. } if path == "xxx/subdir/doc"));
    }

    #[test]
    fn rm_modes_per_matrix() {
        let c = unwrap_ac(
            Command::Rm {
                path: "a".into(),
                recursive: false,
            },
            Some("c"),
        );
        assert!(matches!(c, Command::Rm { path, .. } if path == "c/a"));
        let c = unwrap_ac(
            Command::Rm {
                path: "a".into(),
                recursive: true,
            },
            Some("c"),
        );
        assert!(matches!(c, Command::Rm { path, .. } if path == "c/a"));
        let c = unwrap_ac(
            Command::Rm {
                path: "f*".into(),
                recursive: false,
            },
            Some("c"),
        );
        assert!(matches!(c, Command::Rm { path, .. } if path == "c/f*"));
        let c = unwrap_ac(
            Command::Rm {
                path: "cat".into(),
                recursive: false,
            },
            None,
        );
        assert!(matches!(c, Command::Rm { path, .. } if path == "cat"));
    }

    #[test]
    fn rm_trailing_slash_relative_uses_cwd() {
        let c = unwrap_ac(
            Command::Rm {
                path: "sub/".into(),
                recursive: false,
            },
            Some("notes"),
        );
        assert!(matches!(c, Command::Rm { path, .. } if path == "notes/sub"));
    }

    #[test]
    fn mv_bare_uses_cwd_at_root_directory_rename() {
        let c = unwrap_ac(
            Command::Mv {
                src: "a".into(),
                dst: "b".into(),
            },
            Some("z"),
        );
        assert!(matches!(c, Command::Mv { src, dst } if src == "z/a" && dst == "z/b"));
        let c = unwrap_ac(
            Command::Mv {
                src: "a".into(),
                dst: "b".into(),
            },
            None,
        );
        assert!(matches!(c, Command::Mv { src, dst } if src == "a" && dst == "b"));
    }

    #[test]
    fn mv_bare_glob_prepended_with_cwd() {
        let c = unwrap_ac(
            Command::Mv {
                src: "p*".into(),
                dst: "q".into(),
            },
            Some("z"),
        );
        assert!(matches!(c, Command::Mv { src, dst } if src == "z/p*" && dst == "z/q"));
    }

    #[test]
    fn mv_dst_trailing_slash_prepends_cwd() {
        let c = unwrap_ac(
            Command::Mv {
                src: "meeting-test".into(),
                dst: "meetings/".into(),
            },
            Some("eva"),
        );
        assert!(matches!(c, Command::Mv { dst, .. } if dst == "eva/meetings"));
    }

    #[test]
    fn path_first_commands_resolve() {
        for (cmd, expect) in [
            (
                Command::Put {
                    path: "d".into(),
                    file: None,
                },
                "c/d",
            ),
            (
                Command::Append {
                    path: "d".into(),
                    file: None,
                },
                "c/d",
            ),
            (
                Command::Head {
                    path: "d".into(),
                    lines: 1,
                    raw: false,
                },
                "c/d",
            ),
            (
                Command::Tail {
                    path: "d".into(),
                    tail: TailMode::Last(1),
                    follow: false,
                    raw: false,
                },
                "c/d",
            ),
            (
                Command::Chat {
                    path: "d".into(),
                    nickname: None,
                    raw: false,
                },
                "c/d",
            ),
            (
                Command::Slice {
                    path: "d".into(),
                    start: 1,
                    end: 2,
                    raw: false,
                },
                "c/d",
            ),
            (Command::Stat { path: "d".into() }, "c/d"),
            (Command::Wc { path: "d".into() }, "c/d"),
            (Command::Edit { path: "d".into() }, "c/d"),
            (Command::Wait { path: "d".into() }, "c/d"),
            (Command::Less { path: "d".into() }, "c/d"),
            (
                Command::Desc {
                    path: "d".into(),
                    description: vec![],
                },
                "c/d",
            ),
        ] {
            let out = unwrap_ac(cmd, Some("c"));
            let (Command::Put { path: p, .. }
            | Command::Append { path: p, .. }
            | Command::Head { path: p, .. }
            | Command::Tail { path: p, .. }
            | Command::Chat { path: p, .. }
            | Command::Slice { path: p, .. }
            | Command::Stat { path: p }
            | Command::Wc { path: p }
            | Command::Edit { path: p }
            | Command::Wait { path: p }
            | Command::Less { path: p }
            | Command::Desc { path: p, .. }) = out
            else {
                panic!("unexpected variant");
            };
            assert_eq!(p, expect);
        }
    }

    #[test]
    fn grep_path_resolves() {
        let c = unwrap_ac(
            Command::Grep {
                pattern: "x".into(),
                path: "p".into(),
                max_matches: 0,
                invert_match: false,
            },
            Some("c"),
        );
        assert!(matches!(c, Command::Grep { path, .. } if path == "c/p"));
    }

    #[test]
    fn reindex_preserves_slash_sentinel_strips_directory_segment() {
        let r = unwrap_ac(Command::Reindex { path: "/".into() }, Some("c"));
        assert!(matches!(r, Command::Reindex { path } if path == "/"));
        let r2 = unwrap_ac(
            Command::Reindex {
                path: "/work".into(),
            },
            Some("c"),
        );
        assert!(matches!(r2, Command::Reindex { path } if path == "work"));
        let r_dot = unwrap_ac(Command::Reindex { path: ".".into() }, Some("c"));
        assert!(matches!(r_dot, Command::Reindex { path } if path == "c"));
    }

    #[test]
    fn search_directory_optional_strips_leading_slash() {
        let s = unwrap_ac(
            Command::Search {
                directory: None,
                query: vec!["q".into()],
            },
            Some("c"),
        );
        assert!(matches!(
            s,
            Command::Search {
                directory: None,
                ..
            }
        ));
        let s2 = unwrap_ac(
            Command::Search {
                directory: Some("/notes".into()),
                query: vec!["q".into()],
            },
            Some("c"),
        );
        assert!(matches!(s2, Command::Search { directory: Some(ref x), .. } if x == "notes"));
        let s3 = unwrap_ac(
            Command::Search {
                directory: Some("/".into()),
                query: vec!["q".into()],
            },
            Some("c"),
        );
        assert!(matches!(s3, Command::Search { directory: Some(ref x), .. } if x == "/"));
    }

    #[test]
    fn search_dot_directory_resolves_cwd() {
        let s = unwrap_ac(
            Command::Search {
                directory: Some(".".into()),
                query: vec!["q".into()],
            },
            Some("eva"),
        );
        assert!(matches!(s, Command::Search { directory: Some(ref x), .. } if x == "eva"));
    }

    #[test]
    fn find_import_export_strip_leading_slash_keep_sentinel() {
        let f = unwrap_ac(
            Command::Find {
                directory: Some("/notes".into()),
                name: None,
            },
            Some("x"),
        );
        assert!(matches!(f, Command::Find { directory: Some(ref s), .. } if s == "notes"));
        let f2 = unwrap_ac(
            Command::Find {
                directory: Some("/".into()),
                name: None,
            },
            None,
        );
        assert!(matches!(f2, Command::Find { directory: Some(ref s), .. } if s == "/"));

        let im = unwrap_ac(
            Command::Import {
                directory: "/c".into(),
                dir: "d".into(),
                name: None,
            },
            Some("x"),
        );
        assert!(matches!(im, Command::Import { directory, .. } if directory == "c"));
        let im2 = unwrap_ac(
            Command::Import {
                directory: "/".into(),
                dir: "d".into(),
                name: None,
            },
            Some("x"),
        );
        assert!(matches!(im2, Command::Import { directory, .. } if directory == "/"));

        let ex = unwrap_ac(
            Command::Export {
                directory: "/c".into(),
                destination: None,
            },
            Some("x"),
        );
        assert!(matches!(ex, Command::Export { directory, .. } if directory == "c"));
        let ex_root = unwrap_ac(
            Command::Export {
                directory: "/".into(),
                destination: None,
            },
            Some("x"),
        );
        assert!(matches!(ex_root, Command::Export { directory, .. } if directory == "/"));

        let im_dot = unwrap_ac(
            Command::Import {
                directory: ".".into(),
                dir: "d".into(),
                name: None,
            },
            Some("eva"),
        );
        assert!(matches!(im_dot, Command::Import { directory, .. } if directory == "eva"));
        let im_rel = unwrap_ac(
            Command::Import {
                directory: "sub".into(),
                dir: "d".into(),
                name: None,
            },
            Some("eva"),
        );
        assert!(matches!(im_rel, Command::Import { directory, .. } if directory == "eva/sub"));

        let ex_dot = unwrap_ac(
            Command::Export {
                directory: ".".into(),
                destination: None,
            },
            Some("eva"),
        );
        assert!(matches!(ex_dot, Command::Export { directory, .. } if directory == "eva"));

        let mk = unwrap_ac(
            Command::Mkdir {
                parents: false,
                path: "/newcat".into(),
                description: None,
            },
            Some("x"),
        );
        assert!(matches!(mk, Command::Mkdir { path, .. } if path == "newcat"));
    }

    // --- absolute `/`-prefix bypass tests ---

    #[test]
    fn ls_slash_at_root_lists_repository_root() {
        let c = unwrap_ac(
            Command::Ls {
                directory: Some("/".into()),
                sort_by_time: false,
                reverse: false,
            },
            None,
        );
        assert!(matches!(c, Command::Ls { directory: Some(ref s), .. } if s == "/"));
    }

    #[test]
    fn ls_slash_inside_cwd_lists_repository_root() {
        let c = unwrap_ac(
            Command::Ls {
                directory: Some("/".into()),
                sort_by_time: false,
                reverse: false,
            },
            Some("notes"),
        );
        assert!(matches!(c, Command::Ls { directory: Some(ref s), .. } if s == "/"));
    }

    #[test]
    fn ls_slash_other_strips_prefix() {
        let c = unwrap_ac(
            Command::Ls {
                directory: Some("/other".into()),
                sort_by_time: false,
                reverse: false,
            },
            Some("notes"),
        );
        assert!(matches!(c, Command::Ls { directory: Some(ref s), .. } if s == "other"));
    }

    #[test]
    fn cat_absolute_path_strips_leading_slash() {
        // `cat /notes/doc` → resolves to "notes/doc"
        let c = unwrap_ac(
            Command::Cat {
                path: "/notes/doc".into(),
                raw: false,
            },
            Some("other"),
        );
        assert!(matches!(c, Command::Cat { path, .. } if path == "notes/doc"));
    }

    #[test]
    fn cat_absolute_path_no_cwd_strips_leading_slash() {
        let c = unwrap_ac(
            Command::Cat {
                path: "/notes/doc".into(),
                raw: false,
            },
            None,
        );
        assert!(matches!(c, Command::Cat { path, .. } if path == "notes/doc"));
    }

    #[test]
    fn rm_absolute_doc_strips_leading_slash() {
        let c = unwrap_ac(
            Command::Rm {
                path: "/cat/doc".into(),
                recursive: false,
            },
            Some("other"),
        );
        assert!(matches!(c, Command::Rm { path, .. } if path == "cat/doc"));
    }

    #[test]
    fn rm_recursive_absolute_cat_strips_leading_slash() {
        let c = unwrap_ac(
            Command::Rm {
                path: "/cat".into(),
                recursive: true,
            },
            Some("other"),
        );
        assert!(matches!(c, Command::Rm { path, .. } if path == "cat"));
    }

    #[test]
    fn put_absolute_path_strips_leading_slash() {
        let c = unwrap_ac(
            Command::Put {
                path: "/cat/doc".into(),
                file: None,
            },
            Some("other"),
        );
        assert!(matches!(c, Command::Put { path, .. } if path == "cat/doc"));
    }

    #[test]
    fn path_first_commands_absolute_slash_strips_with_cwd() {
        for (cmd, expect) in [
            (
                Command::Append {
                    path: "/c/d".into(),
                    file: None,
                },
                "c/d",
            ),
            (
                Command::Head {
                    path: "/c/d".into(),
                    lines: 1,
                    raw: false,
                },
                "c/d",
            ),
            (
                Command::Tail {
                    path: "/c/d".into(),
                    tail: TailMode::Last(1),
                    follow: false,
                    raw: false,
                },
                "c/d",
            ),
            (
                Command::Chat {
                    path: "/c/d".into(),
                    nickname: None,
                    raw: false,
                },
                "c/d",
            ),
            (
                Command::Slice {
                    path: "/c/d".into(),
                    start: 1,
                    end: 2,
                    raw: false,
                },
                "c/d",
            ),
            (
                Command::Stat {
                    path: "/c/d".into(),
                },
                "c/d",
            ),
            (
                Command::Wc {
                    path: "/c/d".into(),
                },
                "c/d",
            ),
            (
                Command::Edit {
                    path: "/c/d".into(),
                },
                "c/d",
            ),
            (
                Command::Wait {
                    path: "/c/d".into(),
                },
                "c/d",
            ),
            (
                Command::Less {
                    path: "/c/d".into(),
                },
                "c/d",
            ),
        ] {
            let out = unwrap_ac(cmd, Some("other"));
            let (Command::Append { path: p, .. }
            | Command::Head { path: p, .. }
            | Command::Tail { path: p, .. }
            | Command::Chat { path: p, .. }
            | Command::Slice { path: p, .. }
            | Command::Stat { path: p }
            | Command::Wc { path: p }
            | Command::Edit { path: p }
            | Command::Wait { path: p }
            | Command::Less { path: p }) = out
            else {
                panic!("unexpected variant");
            };
            assert_eq!(p, expect);
        }
    }

    #[test]
    fn grep_and_mv_absolute_paths_strip() {
        let g = unwrap_ac(
            Command::Grep {
                pattern: "p".into(),
                path: "/c/d".into(),
                max_matches: 0,
                invert_match: false,
            },
            Some("z"),
        );
        assert!(matches!(g, Command::Grep { path, .. } if path == "c/d"));
        let m = unwrap_ac(
            Command::Mv {
                src: "/a/b".into(),
                dst: "/c/d".into(),
            },
            Some("z"),
        );
        assert!(matches!(m, Command::Mv { src, dst } if src == "a/b" && dst == "c/d"));
    }
}

#[cfg(test)]
mod completion_tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::{CompletionState, TbHelper};
    use tabularium::rpc::Client;

    fn make_helper(state: CompletionState) -> (TbHelper, tokio::runtime::Runtime) {
        let client = Arc::new(Mutex::new(
            Client::init("http://127.0.0.1:0", Duration::from_millis(1)).unwrap(),
        ));
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let handle = rt.handle().clone();
        let helper = TbHelper::new(client, handle, Arc::new(Mutex::new(state)));
        (helper, rt)
    }

    #[test]
    fn cd_completes_all_root_directories_on_empty() {
        let state =
            CompletionState::from_test_data(vec!["smg".into(), "notes".into()], vec![], None);
        let (h, _rt) = make_helper(state);
        let mut st = h.state.lock().unwrap();
        let pairs = h.complete_root_directories(&mut st, "");
        let names: Vec<&str> = pairs.iter().map(|p| p.display.as_str()).collect();
        assert!(names.contains(&"smg"));
        assert!(names.contains(&"notes"));
    }

    #[test]
    fn cd_completes_root_directory_with_prefix() {
        let state =
            CompletionState::from_test_data(vec!["smg".into(), "notes".into()], vec![], None);
        let (h, _rt) = make_helper(state);
        let mut st = h.state.lock().unwrap();
        let pairs = h.complete_root_directories(&mut st, "sm");
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].display, "smg");
    }

    #[test]
    fn complete_root_directories_leading_slash_strips_for_match_preserves_in_replacement() {
        let state =
            CompletionState::from_test_data(vec!["smg".into(), "notes".into()], vec![], None);
        let (h, _rt) = make_helper(state);
        let mut st = h.state.lock().unwrap();
        let pairs = h.complete_root_directories(&mut st, "/sm");
        assert_eq!(pairs.len(), 1, "leading slash must not prevent matches");
        assert_eq!(pairs[0].display, "smg");
        assert!(
            pairs[0].replacement.starts_with('/'),
            "replacement must keep leading slash"
        );
    }

    #[test]
    fn complete_path_leading_slash_root_prefix_yields_results() {
        let state = CompletionState::from_test_data(
            vec!["smg".into(), "notes".into()],
            vec![("/", vec![("smg", true), ("notes", true)])],
            None,
        );
        let (h, _rt) = make_helper(state);
        let mut st = h.state.lock().unwrap();
        let pairs = h.complete_path(&mut st, "/smg");
        assert!(
            !pairs.is_empty(),
            "leading-slash root-directory prefix must yield completions"
        );
        assert!(
            pairs.iter().all(|p| p.replacement.starts_with('/')),
            "all replacements must keep leading slash"
        );
    }

    #[test]
    fn complete_path_leading_slash_doc_prefix_yields_results() {
        let state = CompletionState::from_test_data(
            vec!["smg".into()],
            vec![("/smg", vec![("doc1", false), ("doc2", false)])],
            None,
        );
        let (h, _rt) = make_helper(state);
        let mut st = h.state.lock().unwrap();
        let pairs = h.complete_path(&mut st, "/smg/doc");
        assert!(
            !pairs.is_empty(),
            "leading-slash doc path must yield completions"
        );
        assert!(
            pairs.iter().all(|p| p.display.starts_with('/')),
            "displays must keep leading slash"
        );
        assert!(
            pairs.iter().all(|p| p.replacement.starts_with('/')),
            "replacements must keep leading slash"
        );
    }

    #[test]
    fn complete_path_plain_prefix_no_cwd() {
        let state = CompletionState::from_test_data(
            vec!["smg".into(), "notes".into()],
            vec![("/", vec![("smg", true), ("notes", true)])],
            None,
        );
        let (h, _rt) = make_helper(state);
        let mut st = h.state.lock().unwrap();
        let pairs = h.complete_path(&mut st, "sm");
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].display, "smg/");
        assert_eq!(pairs[0].replacement, "smg");
    }

    #[test]
    fn cd_completion_lists_directories_only() {
        use rustyline::completion::Completer;
        use rustyline::{Context, history::DefaultHistory};

        // Non-empty `root_dir_names` keeps `ensure_root_directories` from hitting RPC in tests.
        let state = CompletionState::from_test_data(
            vec!["_".into()],
            vec![("/", vec![("work", true), ("readme.md", false)])],
            None,
        );
        let (h, _rt) = make_helper(state);
        let hist = DefaultHistory::new();
        let ctx = Context::new(&hist);
        let r = h.complete("cd ", 3, &ctx).unwrap();
        let (_, pairs) = r;
        assert!(
            pairs.iter().any(|p| p.display.starts_with("work")),
            "expected directory entry"
        );
        assert!(
            !pairs.iter().any(|p| p.display.contains("readme")),
            "files must not appear in cd completion"
        );
    }

    #[test]
    fn mv_after_src_space_completes_empty_partial() {
        use rustyline::completion::Completer;
        use rustyline::{Context, history::DefaultHistory};

        let state = CompletionState::from_test_data(
            vec!["_".into()],
            vec![("/", vec![("src", true), ("dst", true)])],
            None,
        );
        let (h, _rt) = make_helper(state);
        let line = "mv src ";
        let hist = DefaultHistory::new();
        let ctx = Context::new(&hist);
        let r = h.complete(line, line.len(), &ctx).unwrap();
        let (_, pairs) = r;
        let displays: Vec<&str> = pairs.iter().map(|p| p.display.as_str()).collect();
        assert!(
            displays.contains(&"dst/"),
            "destination candidates expected, got {displays:?}"
        );
    }
}

#[cfg(test)]
mod cd_and_completion_prefix_tests {
    use super::{merge_cd_path, shell_completion_parent_prefix};

    #[test]
    fn merge_cd_relative_extends_cwd() {
        assert_eq!(merge_cd_path("b", Some("a")).unwrap(), "/a/b");
    }

    #[test]
    fn merge_cd_relative_at_root() {
        assert_eq!(merge_cd_path("b", None).unwrap(), "/b");
    }

    #[test]
    fn merge_cd_absolute_ignores_cwd() {
        assert_eq!(merge_cd_path("/x", Some("a")).unwrap(), "/x");
    }

    #[test]
    fn merge_cd_strips_trailing_slash() {
        assert_eq!(merge_cd_path("sub/", Some("a")).unwrap(), "/a/sub");
    }

    #[test]
    fn merge_cd_root_only() {
        assert_eq!(merge_cd_path("/", None).unwrap(), "/");
    }

    #[test]
    fn merge_cd_dot_targets_cwd() {
        assert_eq!(merge_cd_path(".", Some("a/b")).unwrap(), "/a/b");
        assert_eq!(merge_cd_path(".", None).unwrap(), "/");
    }

    #[test]
    fn completion_trailing_slash_lists_directory_children() {
        let (parent, pfx) = shell_completion_parent_prefix("chats/", None).unwrap();
        assert_eq!(parent, "/chats");
        assert_eq!(pfx, "");
    }
}

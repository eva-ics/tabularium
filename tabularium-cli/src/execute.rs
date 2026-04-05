//! Shared command execution for one-shot CLI and interactive shell.

use std::fmt::Write as _;
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::time::UNIX_EPOCH;

use filetime::{FileTime, set_file_mtime};
use globset::GlobBuilder;
use regex::Regex;
use tabularium::Error;
use tabularium::TailMode;
use tabularium::Timestamp;
use tabularium::resource_path::{normalize_user_path, parent_and_final_name};
use tabularium::rpc::{Client, ListedEntryRow, SearchHitRow, StatRow};
use tabularium::validate_chat_speaker_id;
use tabularium::validate_entity_name;
use tabularium::ws::RecvMessage;
use tokio::io::AsyncBufReadExt;

use crate::Command;
use crate::render::mad_skin;

pub(crate) type BoxErr = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ChatSubmitAction {
    Ignore,
    Exit,
    Edit,
    /// Full-document `$EDITOR` flow; `None` = current chat path.
    EditFullDocument {
        path: Option<String>,
    },
    History,
    ChangeNick(String),
    Send(String),
}

pub(crate) fn parse_chat_submit(message: &str) -> Result<ChatSubmitAction, String> {
    let message = message.trim_end_matches(['\r', '\n']);
    if message.is_empty() {
        return Ok(ChatSubmitAction::Ignore);
    }

    if message.contains('\n') {
        Ok(ChatSubmitAction::Send(message.to_owned()))
    } else {
        let command = message.trim();
        if let Some(rest) = command.strip_prefix('/') {
            let (verb, args) = match rest.split_once(char::is_whitespace) {
                Some((verb, args)) => (verb, args.trim()),
                None => (rest, ""),
            };

            match verb.to_ascii_lowercase().as_str() {
                "q" | "quit" | "exit" if args.is_empty() => Ok(ChatSubmitAction::Exit),
                "e" | "edit" if args.is_empty() => Ok(ChatSubmitAction::Edit),
                "d" | "doc" if args.is_empty() => {
                    Ok(ChatSubmitAction::EditFullDocument { path: None })
                }
                "d" | "doc" => Ok(ChatSubmitAction::EditFullDocument {
                    path: Some(args.to_string()),
                }),
                "h" | "history" if args.is_empty() => Ok(ChatSubmitAction::History),
                "h" | "history" => Err(format!("/{verb} does not take arguments")),
                "q" | "quit" | "exit" | "e" | "edit" => {
                    Err(format!("/{verb} does not take arguments"))
                }
                "nick" if args.is_empty() => Err("/nick requires a nickname".into()),
                "nick" => {
                    validate_chat_speaker_id(args).map_err(|e| e.to_string())?;
                    Ok(ChatSubmitAction::ChangeNick(args.to_owned()))
                }
                _ => Err(format!("unknown chat command: /{verb}")),
            }
        } else {
            Ok(ChatSubmitAction::Send(message.to_owned()))
        }
    }
}

/// Whether the command runs from the interactive shell or one-shot CLI.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecuteContext {
    Cli,
    Shell,
}

/// Per-command `--raw` (cat/head/tail/slice/chat) plus derived TTY policy for markdown output.
#[derive(Clone, Copy)]
pub(crate) struct ExecuteOpts {
    pub(crate) raw: bool,
}

/// Applies `ls -t` / `ls -r` ordering in place; leaves server `kind, name` order when both flags are off.
/// `-t` alone: newest `modified_at` first, `name` ascending as tie-breaker. `-tr`: oldest first (reversed).
fn sort_ls_rows(rows: &mut [ListedEntryRow], sort_by_time: bool, reverse: bool) {
    if !sort_by_time && !reverse {
        return;
    }
    rows.sort_by(|a, b| {
        let cmp = if sort_by_time {
            b.modified_at
                .cmp(&a.modified_at)
                .then_with(|| a.name().cmp(b.name()))
        } else {
            a.name().cmp(b.name())
        };
        if reverse { cmp.reverse() } else { cmp }
    });
}

fn stat_row_to_listed_row(s: &StatRow) -> ListedEntryRow {
    ListedEntryRow {
        id: s.id(),
        kind: 1,
        name: s.name().to_string(),
        description: None,
        created_at: s.created_at(),
        modified_at: s.modified_at(),
        accessed_at: s.accessed_at(),
        size_bytes: Some(s.size_bytes()),
        recursive_file_count: 0,
    }
}

/// Wildcards only in the final segment; parent path must be literal.
async fn expand_final_segment_glob(
    client: &Client,
    normalized_path: &str,
) -> Result<Option<Vec<(String, ListedEntryRow)>>, BoxErr> {
    if normalized_path == "/" {
        return Ok(None);
    }
    let (parent, last) = normalized_path
        .rsplit_once('/')
        .map(|(p, n)| {
            let p = if p.is_empty() {
                "/".to_string()
            } else {
                p.to_string()
            };
            (p, n.to_string())
        })
        .ok_or_else(|| -> BoxErr { "invalid path".into() })?;
    if !path_has_glob_metachar(&last) {
        return Ok(None);
    }
    if path_has_glob_metachar(&parent) {
        return Err("wildcards are only allowed in the final path segment".into());
    }
    let matcher = compile_name_glob(&last)?;
    let rows = client.list_directory(&parent).await?;
    let mut out: Vec<_> = rows
        .into_iter()
        .filter(|e| matcher.is_match(e.name()))
        .map(|e| (join_abs_dir_entry(&parent, e.name()), e))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    if out.is_empty() {
        return Err(format!("no entries match {normalized_path}").into());
    }
    Ok(Some(out))
}

async fn resolve_read_file_paths(client: &Client, user_path: &str) -> Result<Vec<String>, BoxErr> {
    let norm =
        normalize_user_path(user_path.trim()).map_err(|e| -> BoxErr { e.to_string().into() })?;
    if let Some(entries) = expand_final_segment_glob(client, &norm).await? {
        let mut paths: Vec<String> = entries
            .into_iter()
            .filter(|(_, r)| r.is_file())
            .map(|(p, _)| p)
            .collect();
        paths.sort();
        if paths.is_empty() {
            return Err(format!("no files match {user_path}").into());
        }
        return Ok(paths);
    }
    Ok(vec![norm])
}

fn print_ls_rows(rows: &[ListedEntryRow]) {
    if io::stdout().is_terminal() {
        let table_rows: Vec<Vec<String>> = rows
            .iter()
            .map(|r| {
                let kind = if r.is_directory() { "dir" } else { "file" };
                let name_cell = tty_style_dir_name(r.name(), r.is_directory());
                let size = if r.is_file() {
                    r.size_bytes.unwrap_or(0).to_string()
                } else {
                    r.recursive_file_count().to_string()
                };
                let desc = r.description().unwrap_or("");
                vec![
                    kind.to_string(),
                    name_cell,
                    size,
                    ts_rfc3339(r.modified_at),
                    desc.to_string(),
                ]
            })
            .collect();
        print_cli_table(
            true,
            &["kind", "name", "size_or_files", "modified", "description"],
            &table_rows,
        );
    } else {
        for r in rows {
            let kind = if r.is_directory() { "dir" } else { "file" };
            let size = if r.is_file() {
                r.size_bytes.unwrap_or(0).to_string()
            } else {
                r.recursive_file_count().to_string()
            };
            println!(
                "{}\t{}\t{}\t{}\t{}",
                kind,
                r.name(),
                size,
                ts_rfc3339(r.modified_at),
                r.description().unwrap_or("")
            );
        }
    }
}

async fn run_ls_listing(
    client: &Client,
    directory: Option<String>,
    sort_by_time: bool,
    reverse: bool,
) -> Result<(), BoxErr> {
    let dir_path = match directory.as_deref().map(str::trim) {
        None | Some("") => "/".to_string(),
        Some(p) => normalize_user_path(p).map_err(|e| -> BoxErr { e.to_string().into() })?,
    };

    if let Some(entries) = expand_final_segment_glob(client, &dir_path).await? {
        let mut rows: Vec<ListedEntryRow> = entries.into_iter().map(|(_, r)| r).collect();
        sort_ls_rows(&mut rows, sort_by_time, reverse);
        print_ls_rows(&rows);
        return Ok(());
    }

    if client.document_exists(&dir_path).await? {
        let stat = client.document_stat(&dir_path).await?;
        let row = stat_row_to_listed_row(&stat);
        print_ls_rows(std::slice::from_ref(&row));
        return Ok(());
    }

    let mut rows = client.list_directory(&dir_path).await?;
    sort_ls_rows(&mut rows, sort_by_time, reverse);
    print_ls_rows(&rows);
    Ok(())
}

pub(crate) fn execute_opts_from_command(cmd: &Command) -> ExecuteOpts {
    let raw = match cmd {
        Command::Cat { raw, .. }
        | Command::Head { raw, .. }
        | Command::Tail { raw, .. }
        | Command::Slice { raw, .. }
        | Command::Chat { raw, .. } => *raw,
        _ => false,
    };
    ExecuteOpts { raw }
}

fn stdout_forces_raw_document(opts: ExecuteOpts) -> bool {
    opts.raw || !io::stdout().is_terminal()
}

/// Cat / head / tail / slice / chat stream: markdown on TTY unless `raw` or piped.
pub(crate) fn write_document_stdout(content: &str, opts: ExecuteOpts) -> Result<(), BoxErr> {
    if stdout_forces_raw_document(opts) {
        print!("{content}");
    } else {
        let skin = mad_skin();
        skin.write_text(content)
            .map_err(|e| -> BoxErr { e.to_string().into() })?;
    }
    io::stdout().flush()?;
    Ok(())
}

fn finish_document_output(content: &str, opts: ExecuteOpts) -> Result<(), BoxErr> {
    write_document_stdout(content, opts)?;
    if !content.is_empty() && !content.ends_with('\n') {
        println!();
    }
    Ok(())
}

fn use_ansi_stdout() -> bool {
    io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

/// Bold white score, yellow path, bold white line number; snippet already carries query highlights.
fn format_search_hit_line_tty(h: &SearchHitRow, snippet_ansi: &str) -> String {
    let score_txt = format!("{:.4}", h.score())
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string();
    let score = format!("\x1b[1;37m{score_txt}\x1b[0m");
    let path_meta = match h.line_number() {
        Some(n) => format!("\x1b[33m{}\x1b[0m:\x1b[1;37m{n}\x1b[0m", h.path()),
        None => format!("\x1b[33m{}\x1b[0m", h.path()),
    };
    format!("{score}  {path_meta}  {snippet_ansi}")
}

fn merge_intervals(mut ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    ranges.sort_by_key(|r| r.0);
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (s, e) in ranges {
        if s >= e {
            continue;
        }
        match out.last_mut() {
            Some((_ls, le)) if s <= *le => *le = (*le).max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

fn highlight_query_in_snippet_tty(snippet: &str, query: &str) -> String {
    let display = snippet_cli_display(snippet);
    let lower = display.to_lowercase();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for tok in query.split_whitespace() {
        if tok.is_empty() {
            continue;
        }
        let needle = tok.to_lowercase();
        let mut start = 0usize;
        while let Some(rel) = lower[start..].find(needle.as_str()) {
            let s = start + rel;
            let e = s + needle.len();
            ranges.push((s, e));
            start = e;
        }
    }
    if ranges.is_empty() {
        return display;
    }
    let merged = merge_intervals(ranges);
    let mut out = String::new();
    let mut last = 0usize;
    for (s, e) in merged {
        out.push_str(&display[last..s]);
        out.push_str("\x1b[1;33m");
        out.push_str(&display[s..e]);
        out.push_str("\x1b[0m");
        last = e;
    }
    out.push_str(&display[last..]);
    out
}

fn grep_line_with_matches_tty(line_no: usize, text: &str, re: &Regex) -> String {
    let prefix = format!("{line_no:04}: ");
    let mut out = prefix.clone();
    let mut last = 0usize;
    for m in re.find_iter(text) {
        out.push_str(&text[last..m.start()]);
        out.push_str("\x1b[1;33m");
        out.push_str(m.as_str());
        out.push_str("\x1b[0m");
        last = m.end();
    }
    out.push_str(&text[last..]);
    out.push('\n');
    out
}

/// RPC endpoint + timeout for spawning a nested `tb` from `tb shell` (e.g. `wait` subprocess).
pub(crate) struct ShellChildRpc {
    api_uri: String,
    timeout_sec: u64,
    /// Exact `--header` strings from the parent CLI (child merges TB_HEADERS then these).
    header_flags: Vec<String>,
}

impl ShellChildRpc {
    pub(crate) fn new(api_uri: String, timeout_sec: u64, header_flags: Vec<String>) -> Self {
        Self {
            api_uri,
            timeout_sec,
            header_flags,
        }
    }

    pub(crate) fn timeout_sec(&self) -> u64 {
        self.timeout_sec
    }

    pub(crate) fn set_timeout_sec(&mut self, sec: u64) {
        self.timeout_sec = sec;
    }

    /// Apply parent's `-u`, `-t`, optional `--config`, and repeated `--header` before subcommand args.
    pub(crate) fn prepend_tb_globals(&self, cmd: &mut StdCommand) {
        cmd.arg("-u").arg(&self.api_uri);
        cmd.arg("-t").arg(self.timeout_sec.max(1).to_string());
        for h in &self.header_flags {
            cmd.arg("--header").arg(h);
        }
    }
}

/// Parent ignores Ctrl-C while waiting so SIGINT reaches a foreground child (`wait`, `!` shell, etc.).
#[cfg(unix)]
pub(crate) struct SigIntIgnoreWhileWaiting {
    previous: libc::sighandler_t,
}

#[cfg(unix)]
impl SigIntIgnoreWhileWaiting {
    pub(crate) fn new() -> Result<Self, BoxErr> {
        unsafe {
            let previous = libc::signal(libc::SIGINT, libc::SIG_IGN);
            if previous == libc::SIG_ERR {
                return Err("could not ignore SIGINT for subprocess".into());
            }
            Ok(Self { previous })
        }
    }
}

#[cfg(unix)]
impl Drop for SigIntIgnoreWhileWaiting {
    fn drop(&mut self) {
        unsafe {
            libc::signal(libc::SIGINT, self.previous);
        }
    }
}

fn ws_tail_print_msg(
    msg: &RecvMessage,
    suppress_resync_banner: bool,
    opts: ExecuteOpts,
) -> Result<(), BoxErr> {
    match msg {
        RecvMessage::Append { data: Some(d), .. } => {
            write_document_stdout(d, opts)?;
        }
        RecvMessage::Reset { data: Some(d), .. } => {
            if !suppress_resync_banner {
                eprintln!("==> document rewritten; resyncing tail <==");
            }
            write_document_stdout(d, opts)?;
            if !d.is_empty() && !d.ends_with('\n') {
                println!();
            }
        }
        RecvMessage::Append { data: None, .. } | RecvMessage::Reset { data: None, .. } => {}
        RecvMessage::Error { message } => {
            return Err(message
                .clone()
                .unwrap_or_else(|| "unknown error".into())
                .into());
        }
        RecvMessage::Unknown { op } => {
            return Err(format!("unexpected ws op: {op}").into());
        }
    }
    Ok(())
}

fn api_host_for_prompt(api_base: &str) -> String {
    let base = api_base.trim_end_matches('/');
    base.strip_prefix("http://")
        .or_else(|| base.strip_prefix("https://"))
        .unwrap_or(base)
        .to_string()
}

fn default_chat_nickname() -> String {
    #[cfg(windows)]
    {
        std::env::var("USERNAME").unwrap_or_else(|_| "user".to_string())
    }
    #[cfg(not(windows))]
    {
        std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_else(|_| "user".to_string())
    }
}

async fn cmd_tail_follow(
    client: &Client,
    path: &str,
    tail: TailMode,
    opts: ExecuteOpts,
) -> Result<ExecuteOutcome, BoxErr> {
    let mut ws =
        tabularium::ws::Client::connect_with_headers(client.api_base(), client.extra_headers())
            .await?;
    ws.subscribe(path, tail).await?;
    // Keep one SIGINT listener for the whole follow session so Ctrl-C cannot land
    // between loop iterations and get lost while the stream is busy.
    let interrupted = tokio::signal::ctrl_c();
    tokio::pin!(interrupted);
    loop {
        tokio::select! {
            res = ws.recv() => {
                let Some(msg) = res? else {
                    break;
                };
                ws_tail_print_msg(&msg, false, opts)?;
            }
            _ = &mut interrupted => {
                println!("interrupted");
                let _ = ws.close().await;
                return Ok(ExecuteOutcome::Interrupted);
            }
        }
    }
    let _ = ws.close().await;
    Ok(ExecuteOutcome::Ok)
}

/// Full-document WS snapshot on `chat` subscribe (`tail -n +1`); scrollback is paged locally in the TUI.
fn initial_chat_subscribe_tail() -> TailMode {
    TailMode::FromLine(1)
}

async fn cmd_chat(
    client: &tabularium::rpc::Client,
    path: &str,
    nickname: &str,
    opts: ExecuteOpts,
) -> Result<ExecuteOutcome, BoxErr> {
    validate_chat_speaker_id(nickname).map_err(|e| -> BoxErr { e.to_string().into() })?;
    if !client.document_exists(path).await? {
        client.put_document(path, "").await?;
    }
    let tail = initial_chat_subscribe_tail();
    let mut ws =
        tabularium::ws::Client::connect_with_headers(client.api_base(), client.extra_headers())
            .await?;
    ws.subscribe(path, tail).await?;

    let host = api_host_for_prompt(client.api_base());

    if io::stdout().is_terminal() && io::stdin().is_terminal() {
        return crate::chat_tui::run_chat(ws, client, path, nickname, &host, opts).await;
    }

    let mut nickname = nickname.to_owned();

    let Some(first) = ws.recv().await? else {
        let _ = ws.close().await;
        return Ok(ExecuteOutcome::Ok);
    };
    let first_nonempty = matches!(
        &first,
        RecvMessage::Append { data: Some(d), .. } | RecvMessage::Reset { data: Some(d), .. }
            if !d.is_empty()
    );
    if first_nonempty {
        println!();
    }
    ws_tail_print_msg(&first, true, opts)?;

    let stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let mut stdin_lines = stdin.lines();
    let interrupted = tokio::signal::ctrl_c();
    tokio::pin!(interrupted);

    loop {
        print!("tb {host} {path} {nickname} → ");
        io::stdout().flush()?;

        tokio::select! {
            _ = &mut interrupted => {
                println!("interrupted");
                let _ = ws.close().await;
                return Ok(ExecuteOutcome::Interrupted);
            }
            line = stdin_lines.next_line() => {
                let line = line?;
                let Some(line) = line else {
                    break;
                };
                match parse_chat_submit(&line) {
                    Ok(ChatSubmitAction::Ignore) => {}
                    Ok(ChatSubmitAction::Exit) => break,
                    Ok(ChatSubmitAction::Edit) => {
                        eprintln!("/edit requires an interactive terminal");
                    }
                    Ok(ChatSubmitAction::EditFullDocument { path: target }) => {
                        let doc_path = target
                            .as_deref()
                            .unwrap_or(path)
                            .trim();
                        if let Err(e) = chat_edit_full_document(client, doc_path).await {
                            eprintln!("edit document: {e}");
                        }
                    }
                    Ok(ChatSubmitAction::History) => {
                        match client.get_document(path).await {
                            Ok(body) => {
                                if let Err(e) = shell_pager_always(body.content()) {
                                    eprintln!("history: {e}");
                                }
                            }
                            Err(e) => eprintln!("history: {e}"),
                        }
                    }
                    Ok(ChatSubmitAction::ChangeNick(next)) => {
                        nickname = next;
                    }
                    Ok(ChatSubmitAction::Send(message)) => {
                        if let Err(e) = ws.say(path, &nickname, &message).await {
                            eprintln!("say: {e}");
                        }
                    }
                    Err(message) => eprintln!("{message}"),
                }
            }
            res = ws.recv() => {
                let Some(msg) = res? else {
                    break;
                };
                ws_tail_print_msg(&msg, true, opts)?;
            }
        }
    }
    let _ = ws.close().await;
    Ok(ExecuteOutcome::Ok)
}

async fn wait_in_shell_subprocess(path: &str, conn: &ShellChildRpc) -> Result<(), BoxErr> {
    let exe = std::env::current_exe()?;
    let path = path.to_string();

    #[cfg(unix)]
    let _sig_guard = SigIntIgnoreWhileWaiting::new()?;

    let mut cmd = StdCommand::new(&exe);
    cmd.stdin(Stdio::null());
    conn.prepend_tb_globals(&mut cmd);
    cmd.arg("wait").arg(&path);

    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            Ok(())
        });
    }

    let status = tokio::task::spawn_blocking(move || cmd.status())
        .await
        .map_err(|e| -> BoxErr { e.to_string().into() })??;

    if status.success() || status.code() == Some(130) {
        return Ok(());
    }
    Err(format!("wait subprocess exited with status {status}").into())
}

/// Normal completion vs wait interrupted by Ctrl-C (CLI exits 130).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecuteOutcome {
    Ok,
    Interrupted,
}

#[allow(clippy::too_many_lines)]
pub(crate) async fn execute(
    client: &Client,
    command: Command,
    ctx: ExecuteContext,
    shell_child_rpc: Option<&ShellChildRpc>,
    opts: ExecuteOpts,
) -> Result<ExecuteOutcome, BoxErr> {
    match command {
        Command::Shell { .. } => {
            eprintln!("already in interactive shell (type exit to leave)");
        }
        Command::Ls {
            directory,
            sort_by_time,
            reverse,
        } => {
            run_ls_listing(client, directory, sort_by_time, reverse).await?;
        }
        Command::Lt { directory, reverse } => {
            run_ls_listing(client, directory, true, reverse).await?;
        }
        Command::Cat { path, .. } => {
            let paths = resolve_read_file_paths(client, path.trim()).await?;
            for (i, p) in paths.iter().enumerate() {
                if paths.len() > 1 {
                    println!("==> {p} <==");
                }
                let body = client.get_document(p).await?;
                finish_document_output(body.content(), opts)?;
                if i + 1 < paths.len() {
                    println!();
                }
            }
        }
        Command::Search { directory, query } => {
            let q = query.join(" ");
            if q.is_empty() {
                return Err("search: empty query".into());
            }
            let dir = match directory.as_deref().map(str::trim) {
                None | Some("") => None,
                Some(c) => {
                    Some(normalize_user_path(c).map_err(|e| -> BoxErr { e.to_string().into() })?)
                }
            };
            let hits = client
                .search(&q, dir.as_deref().map(std::path::Path::new))
                .await?;
            if use_ansi_stdout() {
                let mut out = String::new();
                for h in &hits {
                    let snip = highlight_query_in_snippet_tty(h.snippet(), &q);
                    let _ = writeln!(out, "{}", format_search_hit_line_tty(h, &snip));
                }
                if shell_pager_if_tall(ctx, &out)? {
                    return Ok(ExecuteOutcome::Ok);
                }
                print!("{out}");
            } else {
                for h in hits {
                    println!(
                        "{}\t{}\t{}",
                        h.score(),
                        search_hit_path_line(&h),
                        snippet_cli_display(h.snippet())
                    );
                }
            }
        }
        Command::Find { directory, name } => {
            cmd_find(client, directory.as_deref(), name.as_deref()).await?;
        }
        Command::Desc { path, description } => {
            let path =
                normalize_user_path(path.trim()).map_err(|e| -> BoxErr { e.to_string().into() })?;
            if description.is_empty() {
                match client.describe_entry(&path).await? {
                    None => {}
                    Some(s) if s.is_empty() => {}
                    Some(s) => println!("{s}"),
                }
            } else {
                let joined = description.join(" ");
                client.set_entry_description(&path, &joined).await?;
            }
        }
        Command::Put { path, file } => {
            let content = read_input(file.as_deref())?;
            let path =
                normalize_user_path(path.trim()).map_err(|e| -> BoxErr { e.to_string().into() })?;
            client.put_document(&path, &content).await?;
        }
        Command::Import {
            directory,
            dir,
            name,
        } => {
            cmd_import(client, &directory, Path::new(&dir), name.as_deref()).await?;
        }
        Command::Less { path } => {
            let path =
                normalize_user_path(path.trim()).map_err(|e| -> BoxErr { e.to_string().into() })?;
            let body = client.get_document(&path).await?;
            shell_pager_always(body.content())?;
        }
        Command::Export {
            directory,
            destination,
        } => {
            let dest = destination
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            cmd_export(client, &directory, &dest).await?;
        }
        Command::Append { path, file } => {
            let content = read_input(file.as_deref())?;
            let path =
                normalize_user_path(path.trim()).map_err(|e| -> BoxErr { e.to_string().into() })?;
            client.append_document(&path, &content).await?;
        }
        Command::Rm { path, recursive } => {
            let path = path.trim().to_string();
            cmd_rm(client, &path, recursive).await?;
        }
        Command::Head { path, lines, .. } => {
            let path =
                normalize_user_path(path.trim()).map_err(|e| -> BoxErr { e.to_string().into() })?;
            let t = client.document_head(&path, lines).await?;
            if shell_pager_if_tall(ctx, &t)? {
                return Ok(ExecuteOutcome::Ok);
            }
            finish_document_output(&t, opts)?;
        }
        Command::Tail {
            path, tail, follow, ..
        } => {
            let path =
                normalize_user_path(path.trim()).map_err(|e| -> BoxErr { e.to_string().into() })?;
            if follow {
                return cmd_tail_follow(client, &path, tail, opts).await;
            }
            let t = client.document_tail(&path, tail).await?;
            if shell_pager_if_tall(ctx, &t)? {
                return Ok(ExecuteOutcome::Ok);
            }
            finish_document_output(&t, opts)?;
        }
        Command::Chat { path, nickname, .. } => {
            let path =
                normalize_user_path(path.trim()).map_err(|e| -> BoxErr { e.to_string().into() })?;
            let nick = nickname.unwrap_or_else(default_chat_nickname);
            return cmd_chat(client, &path, &nick, opts).await;
        }
        Command::Slice {
            path, start, end, ..
        } => {
            let path =
                normalize_user_path(path.trim()).map_err(|e| -> BoxErr { e.to_string().into() })?;
            let t = client.document_slice(&path, start, end).await?;
            if shell_pager_if_tall(ctx, &t)? {
                return Ok(ExecuteOutcome::Ok);
            }
            finish_document_output(&t, opts)?;
        }
        Command::Grep {
            path,
            pattern,
            max_matches,
            invert_match,
        } => {
            let paths = resolve_read_file_paths(client, path.trim()).await?;
            let re = Regex::new(&pattern).map_err(|e| -> BoxErr { format!("grep: {e}").into() })?;
            let multi = paths.len() > 1;
            if io::stdout().is_terminal() {
                let mut out = String::new();
                for p in &paths {
                    let rows = client
                        .document_grep(p, &pattern, max_matches, invert_match)
                        .await?;
                    for r in &rows {
                        let line = if invert_match {
                            if multi {
                                format!("{}:{:04}: {}\n", p, r.line(), r.text())
                            } else {
                                format!("{:04}: {}\n", r.line(), r.text())
                            }
                        } else if multi {
                            let inner = grep_line_with_matches_tty(r.line(), r.text(), &re);
                            format!("{p}:{inner}")
                        } else {
                            grep_line_with_matches_tty(r.line(), r.text(), &re)
                        };
                        out.push_str(&line);
                    }
                }
                if shell_pager_if_tall(ctx, &out)? {
                    return Ok(ExecuteOutcome::Ok);
                }
                print!("{out}");
            } else {
                for p in paths {
                    let rows = client
                        .document_grep(&p, &pattern, max_matches, invert_match)
                        .await?;
                    for r in rows {
                        if multi {
                            println!("{}:{}", p, r.text());
                        } else {
                            println!("{}", r.text());
                        }
                    }
                }
            }
        }
        Command::Stat { path } => {
            let path =
                normalize_user_path(path.trim()).map_err(|e| -> BoxErr { e.to_string().into() })?;
            let s = client.document_stat(&path).await?;
            let table_rows = vec![
                vec!["id".to_string(), s.id().to_string()],
                vec!["path".to_string(), s.path().to_string()],
                vec!["name".to_string(), s.name().to_string()],
                vec!["size_bytes".to_string(), s.size_bytes().to_string()],
                vec!["line_count".to_string(), s.line_count().to_string()],
                vec!["created_at".to_string(), ts_rfc3339(s.created_at())],
                vec!["modified_at".to_string(), ts_rfc3339(s.modified_at())],
                vec!["accessed_at".to_string(), ts_rfc3339(s.accessed_at())],
            ];
            if io::stdout().is_terminal() {
                print_cli_table(true, &["name", "value"], &table_rows);
            } else {
                for row in &table_rows {
                    println!("{}\t{}", row[0], row[1]);
                }
            }
        }
        Command::Test => {
            let t = client.test().await?;
            let uptime_display = if io::stdout().is_terminal() {
                format_uptime_ns(t.uptime())
            } else {
                t.uptime().to_string()
            };
            let rows = [
                ("product_name", t.product_name()),
                ("product_version", t.product_version()),
                ("uptime", uptime_display.as_str()),
            ];
            print_cli_kv_rows(&rows);
        }
        Command::Wc { path } => {
            let paths = resolve_read_file_paths(client, path.trim()).await?;
            let multi = paths.len() > 1;
            if io::stdout().is_terminal() {
                let mut table_rows: Vec<Vec<String>> = Vec::new();
                let mut sum_b = 0u64;
                let mut sum_l = 0usize;
                let mut sum_w = 0usize;
                let mut sum_c = 0usize;
                for p in &paths {
                    let w = client.document_wc(p).await?;
                    sum_b += w.bytes();
                    sum_l += w.lines();
                    sum_w += w.words();
                    sum_c += w.chars();
                    if multi {
                        table_rows.push(vec![
                            p.clone(),
                            w.bytes().to_string(),
                            w.lines().to_string(),
                            w.words().to_string(),
                            w.chars().to_string(),
                        ]);
                    } else {
                        table_rows.push(vec![
                            w.bytes().to_string(),
                            w.lines().to_string(),
                            w.words().to_string(),
                            w.chars().to_string(),
                        ]);
                    }
                }
                if multi {
                    table_rows.push(vec![
                        "total".to_string(),
                        sum_b.to_string(),
                        sum_l.to_string(),
                        sum_w.to_string(),
                        sum_c.to_string(),
                    ]);
                    print_cli_table(
                        true,
                        &["path", "bytes", "lines", "words", "chars"],
                        &table_rows,
                    );
                } else {
                    print_cli_table(true, &["bytes", "lines", "words", "chars"], &table_rows);
                }
            } else {
                for p in &paths {
                    let w = client.document_wc(p).await?;
                    if multi {
                        println!(
                            "{}\t{}\t{}\t{}\t{}",
                            p,
                            w.bytes(),
                            w.lines(),
                            w.words(),
                            w.chars()
                        );
                    } else {
                        println!("{}\t{}\t{}\t{}", w.bytes(), w.lines(), w.words(), w.chars());
                    }
                }
            }
        }
        Command::Edit { path } => {
            let path =
                normalize_user_path(path.trim()).map_err(|e| -> BoxErr { e.to_string().into() })?;
            cmd_edit(client, &path).await?;
        }
        Command::Mkdir { path, description } => {
            let path =
                normalize_user_path(path.trim()).map_err(|e| -> BoxErr { e.to_string().into() })?;
            client
                .create_directory(&path, description.as_deref())
                .await?;
        }
        Command::Touch { path, time } => {
            let path =
                normalize_user_path(path.trim()).map_err(|e| -> BoxErr { e.to_string().into() })?;
            let modified_at = match time.as_deref() {
                None => None,
                Some(s) => Some(
                    tabularium::parse_user_timestamp(s.trim())
                        .map_err(|e| -> BoxErr { e.to_string().into() })?,
                ),
            };
            client.touch_document(&path, modified_at).await?;
        }
        Command::Wait { path } => {
            let path =
                normalize_user_path(path.trim()).map_err(|e| -> BoxErr { e.to_string().into() })?;
            if ctx == ExecuteContext::Shell {
                let Some(conn) = shell_child_rpc else {
                    return Err("internal error: shell wait missing RPC options".into());
                };
                wait_in_shell_subprocess(&path, conn).await?;
            } else {
                tokio::select! {
                    res = client.wait_document(&path) => {
                        res?;
                    }
                    _ = tokio::signal::ctrl_c() => {
                        println!("interrupted");
                        return Ok(ExecuteOutcome::Interrupted);
                    }
                }
            }
        }
        Command::Mv { src, dst } => {
            cmd_mv(client, &src, &dst).await?;
        }
        Command::Reindex { path } => {
            let path =
                normalize_user_path(path.trim()).map_err(|e| -> BoxErr { e.to_string().into() })?;
            let rpc_path = if path == "/" {
                None
            } else {
                Some(std::path::Path::new(path.as_str()))
            };
            client.reindex(rpc_path).await?;
        }
    }
    Ok(ExecuteOutcome::Ok)
}

fn search_hit_path_line(h: &SearchHitRow) -> String {
    let path = h.path();
    match h.line_number() {
        Some(n) => format!("{path}:{n}"),
        None => path.to_string(),
    }
}

fn snippet_cli_display(snippet: &str) -> String {
    snippet
        .chars()
        .map(|c| match c {
            '\n' | '\r' => ' ',
            _ => c,
        })
        .collect()
}

fn should_pager_text_lines(content: &str) -> bool {
    let Some((_, rows)) = terminal_size::terminal_size() else {
        return false;
    };
    let rows = usize::from(rows.0);
    if rows == 0 {
        return false;
    }
    let line_count = content.lines().count().max(1);
    line_count > rows
}

/// TTY: always pipe through `$PAGER` (default `less`). Non-TTY: print raw (no pager).
pub(crate) fn shell_pager_always(text: &str) -> Result<(), BoxErr> {
    if !io::stdout().is_terminal() {
        print!("{text}");
        if !text.ends_with('\n') {
            println!();
        }
        return Ok(());
    }
    let pager = std::env::var("PAGER")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "less".to_string());
    let mut buf: Vec<u8> = text.as_bytes().to_vec();
    if !text.ends_with('\n') {
        buf.push(b'\n');
    }
    if pipe_to_pager(&pager, &buf)? {
        return Ok(());
    }
    print!("{text}");
    if !text.ends_with('\n') {
        println!();
    }
    Ok(())
}

/// Interactive shell + TTY + output taller than the terminal: pipe through `$PAGER` when set.
fn shell_pager_if_tall(ctx: ExecuteContext, text: &str) -> Result<bool, BoxErr> {
    if ctx != ExecuteContext::Shell || !io::stdout().is_terminal() {
        return Ok(false);
    }
    if !should_pager_text_lines(text) {
        return Ok(false);
    }
    let Ok(pager) = std::env::var("PAGER") else {
        return Ok(false);
    };
    if pager.is_empty() {
        return Ok(false);
    }
    let mut buf: Vec<u8> = text.as_bytes().to_vec();
    if !text.ends_with('\n') {
        buf.push(b'\n');
    }
    pipe_to_pager(&pager, &buf)
}

/// Runs `$PAGER` with `content` on stdin. Returns `Ok(true)` if the pager ran.
fn pipe_to_pager(pager: &str, content: &[u8]) -> Result<bool, BoxErr> {
    #[cfg(unix)]
    {
        let mut child = StdCommand::new("sh")
            .arg("-c")
            .arg("exec $PAGER")
            .env("PAGER", pager)
            .stdin(Stdio::piped())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("pager: {e}"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("pager stdin"))?;
        stdin.write_all(content)?;
        drop(stdin);
        let status = child.wait()?;
        if !status.success() {
            return Err(format!("pager exited with {status}").into());
        }
        Ok(true)
    }
    #[cfg(not(unix))]
    {
        let _ = (pager, content);
        Ok(false)
    }
}

#[derive(Eq, PartialEq, Ord, PartialOrd, Clone)]
struct FindRow {
    exact: std::cmp::Reverse<bool>,
    kind: u8,
    label: String,
    modified_at: Timestamp,
    description: Option<String>,
}

fn path_has_glob_metachar(path: &str) -> bool {
    path.chars().any(|c| matches!(c, '*' | '?' | '['))
}

fn compile_name_glob(pat: &str) -> Result<globset::GlobMatcher, BoxErr> {
    Ok(GlobBuilder::new(pat)
        .literal_separator(true)
        .build()
        .map_err(|e| -> BoxErr { format!("invalid glob pattern: {e}").into() })?
        .compile_matcher())
}

async fn cmd_rm(client: &Client, path: &str, recursive: bool) -> Result<(), BoxErr> {
    if !path_has_glob_metachar(path) {
        return cmd_rm_literal(client, path, recursive).await;
    }
    if path.contains('/') {
        if recursive {
            return Err("rm: -r/--recursive with globs: use literal directory path (no wildcards in parent)".into());
        }
        let full =
            normalize_user_path(path.trim()).map_err(|e| -> BoxErr { e.to_string().into() })?;
        let (parent, doc_pat) = full
            .rsplit_once('/')
            .ok_or_else(|| -> BoxErr { "rm: invalid path".into() })?;
        let parent = if parent.is_empty() {
            "/".to_string()
        } else {
            parent.to_string()
        };
        if path_has_glob_metachar(&parent) {
            return Err("rm: parent path must be literal (no wildcards)".into());
        }
        let matcher = compile_name_glob(doc_pat)?;
        let docs = client.list_documents(&parent).await?;
        let mut hits: Vec<String> = docs
            .iter()
            .filter(|d| matcher.is_match(d.name()))
            .map(|d| d.path().to_string())
            .collect();
        hits.sort();
        if hits.is_empty() {
            return Err(format!("rm: no documents match {path}").into());
        }
        for p in hits {
            client.delete_document(&p).await?;
        }
    } else {
        let matcher = compile_name_glob(path)?;
        let entries = client.list_directory("/").await?;
        let mut targets: Vec<String> = entries
            .iter()
            .filter(|e| matcher.is_match(e.name()))
            .map(|e| {
                if e.name().starts_with('/') {
                    e.name().to_string()
                } else {
                    format!("/{}", e.name())
                }
            })
            .collect();
        targets.sort();
        if targets.is_empty() {
            return Err(format!("rm: no entries match {path}").into());
        }
        for p in targets {
            if client.document_exists(&p).await? {
                client.delete_document(&p).await?;
            } else {
                client.delete_directory(&p, recursive).await?;
            }
        }
    }
    Ok(())
}

/// `rm -r /dir` — absolute single segment under root (for recursive directory delete shortcut).
fn rm_absolute_single_segment(path: &str) -> Option<String> {
    let path = path.trim_end_matches('/');
    let rest = path.strip_prefix('/')?;
    if rest.is_empty() || rest.contains('/') {
        return None;
    }
    Some(format!("/{rest}"))
}

async fn cmd_rm_literal(client: &Client, path: &str, recursive: bool) -> Result<(), BoxErr> {
    if recursive && let Some(p) = rm_absolute_single_segment(path) {
        client.delete_directory(&p, true).await?;
        return Ok(());
    }
    let n = normalize_user_path(path.trim_end_matches('/'))
        .map_err(|e| -> BoxErr { e.to_string().into() })?;
    if client.document_exists(&n).await? {
        client.delete_document(&n).await?;
        return Ok(());
    }
    client.delete_directory(&n, recursive).await?;
    Ok(())
}

/// Strip trailing `/` so `mv cat/doc dest/` and `mv cat/doc dest` both work (shell-style directory paths).
fn normalize_mv_segment(path: &str) -> &str {
    path.trim_end_matches('/')
}

/// Join a canonical directory path with a single segment (`/a` + `b` → `/a/b`).
pub(crate) fn join_abs_dir_entry(parent: &str, name: &str) -> String {
    let p = parent.trim_end_matches('/');
    let n = name.trim_start_matches('/');
    if p.is_empty() || p == "/" {
        format!("/{n}")
    } else {
        format!("{p}/{n}")
    }
}

async fn resolve_mv_destination(
    client: &Client,
    src: &str,
    dst_user: &str,
) -> Result<String, BoxErr> {
    let dst_into = dst_user.trim().ends_with('/');
    let dst = normalize_user_path(normalize_mv_segment(dst_user.trim()))
        .map_err(|e| -> BoxErr { e.to_string().into() })?;
    let (_, base) = tabularium::resource_path::parent_and_final_name(src)
        .map_err(|e| -> BoxErr { e.to_string().into() })?;

    if dst_into {
        return Ok(join_abs_dir_entry(&dst, &base));
    }
    if client.document_exists(&dst).await? {
        return Ok(dst);
    }
    if client.list_directory(&dst).await.is_ok() {
        return Ok(join_abs_dir_entry(&dst, &base));
    }
    Ok(dst)
}

async fn cmd_mv(client: &Client, src: &str, dst: &str) -> Result<(), BoxErr> {
    let src = normalize_user_path(normalize_mv_segment(src))
        .map_err(|e| -> BoxErr { e.to_string().into() })?;
    let dst_resolved = resolve_mv_destination(client, &src, dst).await?;
    if client.document_exists(&src).await? {
        client.move_document(&src, &dst_resolved).await?;
        return Ok(());
    }
    let (sp, _) = tabularium::resource_path::parent_and_final_name(&src)
        .map_err(|e| -> BoxErr { e.to_string().into() })?;
    let (dp, dn) = tabularium::resource_path::parent_and_final_name(&dst_resolved)
        .map_err(|e| -> BoxErr { e.to_string().into() })?;
    if sp == dp {
        client.rename_directory(&src, &dst_resolved).await?;
    } else {
        client.move_directory(&src, &dp, &dn).await?;
    }
    Ok(())
}

async fn cmd_find(
    client: &Client,
    directory: Option<&str>,
    name: Option<&str>,
) -> Result<(), BoxErr> {
    let needle_lc = name.filter(|n| !n.is_empty()).map(str::to_lowercase);

    match directory {
        None => {
            let Some(ref n) = needle_lc else {
                return Err(
                    "find: name required (use `find -d DIRECTORY` to list all files under a subtree)"
                        .into(),
                );
            };
            cmd_find_global(client, n).await
        }
        Some(dir_key) => cmd_find_scoped(client, dir_key, needle_lc.as_deref()).await,
    }
}

async fn collect_find_tree(
    client: &Client,
    dir_path: &str,
    rows: &mut Vec<FindRow>,
    needle: Option<&str>,
) -> Result<(), BoxErr> {
    let entries = client.list_directory(dir_path).await?;
    for e in entries {
        let full = join_abs_dir_entry(dir_path, e.name());
        if e.is_directory() {
            let include_dir = match needle {
                None => true,
                Some(n) => e.name().to_lowercase().contains(n),
            };
            if include_dir {
                let exact = needle.is_some_and(|n| e.name().to_lowercase() == n);
                rows.push(FindRow {
                    exact: std::cmp::Reverse(exact),
                    kind: 0,
                    label: full.clone(),
                    modified_at: e.modified_at,
                    description: e.description.clone(),
                });
            }
            Box::pin(collect_find_tree(client, &full, rows, needle)).await?;
        } else if e.is_file() {
            push_find_doc_row(rows, &full, needle, e.modified_at, e.description.clone());
        }
    }
    Ok(())
}

async fn cmd_find_global(client: &Client, n: &str) -> Result<(), BoxErr> {
    let mut rows: Vec<FindRow> = Vec::new();
    collect_find_tree(client, "/", &mut rows, Some(n)).await?;
    rows.sort();
    cmd_find_print(&rows);
    Ok(())
}

async fn cmd_find_scoped(
    client: &Client,
    dir_key: &str,
    needle: Option<&str>,
) -> Result<(), BoxErr> {
    let mut rows: Vec<FindRow> = Vec::new();
    let base = if dir_key == "/" {
        "/".to_string()
    } else {
        normalize_user_path(dir_key.trim()).map_err(|e| -> BoxErr { e.to_string().into() })?
    };
    collect_find_tree(client, &base, &mut rows, needle).await?;
    rows.sort();
    cmd_find_print(&rows);
    Ok(())
}

fn push_find_doc_row(
    rows: &mut Vec<FindRow>,
    doc_path: &str,
    needle: Option<&str>,
    modified_at: Timestamp,
    description: Option<String>,
) {
    let doc_name = doc_path.rsplit_once('/').map_or(doc_path, |(_, n)| n);
    if let Some(n) = needle {
        let dl = doc_name.to_lowercase();
        if !dl.contains(n) {
            return;
        }
    }
    let exact = needle.is_some_and(|n| doc_name.to_lowercase() == n);
    rows.push(FindRow {
        exact: std::cmp::Reverse(exact),
        kind: 1,
        label: doc_path.to_string(),
        modified_at,
        description,
    });
}

fn cmd_find_print(rows: &[FindRow]) {
    if io::stdout().is_terminal() {
        let table_rows: Vec<Vec<String>> = rows
            .iter()
            .map(|r| {
                let kind = if r.kind == 0 { "directory" } else { "file" };
                let path_cell = if r.kind == 0 {
                    tty_style_dir_name(&r.label, true)
                } else {
                    r.label.clone()
                };
                let desc = r.description.as_deref().unwrap_or("");
                vec![
                    kind.to_string(),
                    path_cell,
                    ts_rfc3339(r.modified_at),
                    desc.to_string(),
                ]
            })
            .collect();
        print_cli_table(
            true,
            &["kind", "path", "modified_at", "description"],
            &table_rows,
        );
    } else {
        for r in rows {
            let kind = if r.kind == 0 { "directory" } else { "file" };
            println!(
                "{kind}\t{}\t{}\t{}",
                r.label,
                ts_rfc3339(r.modified_at),
                r.description.as_deref().unwrap_or("")
            );
        }
    }
}

fn timestamp_from_system_time(t: std::time::SystemTime) -> Result<Timestamp, BoxErr> {
    let d = t
        .duration_since(UNIX_EPOCH)
        .map_err(|_| -> BoxErr { "import: filesystem mtime before UNIX epoch".into() })?;
    let nanos = u64::try_from(d.as_nanos())
        .map_err(|_| -> BoxErr { "import: filesystem mtime out of range".into() })?;
    Ok(Timestamp::from_nanos(nanos))
}

fn fs_mtime_timestamp(path: &Path) -> Result<Timestamp, BoxErr> {
    let meta = std::fs::metadata(path).map_err(|e| -> BoxErr { format!("import: {e}").into() })?;
    let t = meta
        .modified()
        .map_err(|e| -> BoxErr { format!("import: {e}").into() })?;
    timestamp_from_system_time(t)
}

async fn server_entry_modified_at(client: &Client, entry_path: &str) -> Result<Timestamp, BoxErr> {
    let (parent, name) =
        parent_and_final_name(entry_path).map_err(|e| -> BoxErr { e.to_string().into() })?;
    let rows = client
        .list_directory(&parent)
        .await
        .map_err(|e| -> BoxErr { e.to_string().into() })?;
    let row = rows
        .iter()
        .find(|r| r.name() == name)
        .ok_or_else(|| -> BoxErr { format!("export: missing {entry_path} in listing").into() })?;
    Ok(row.modified_at)
}

fn apply_fs_mtime_from_timestamp(path: &Path, ts: Timestamp) -> Result<(), BoxErr> {
    let st = UNIX_EPOCH
        .checked_add(ts.as_duration())
        .ok_or_else(|| -> BoxErr { "export: timestamp out of range for SystemTime".into() })?;
    set_file_mtime(path, FileTime::from_system_time(st))
        .map_err(|e| -> BoxErr { format!("export: {e}").into() })?;
    Ok(())
}

fn validate_fs_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("empty name".into());
    }
    if name == "." || name == ".." {
        return Err("reserved name".into());
    }
    if name.chars().any(|c| matches!(c, '/' | '\\' | '\0')) {
        return Err("path separator or NUL in name".into());
    }
    #[cfg(windows)]
    if name.chars().any(|c| "<>:\"|?*".contains(c)) {
        return Err("invalid filesystem character".into());
    }
    Ok(())
}

async fn ensure_directory_path_recursive(client: &Client, path: &str) -> Result<(), BoxErr> {
    let p = normalize_user_path(path.trim()).map_err(|e| -> BoxErr { e.to_string().into() })?;
    if client.list_directory(&p).await.is_ok() {
        return Ok(());
    }
    let segs = tabularium::resource_path::canonical_path_segments(&p)
        .map_err(|e| -> BoxErr { e.to_string().into() })?;
    let mut acc = String::from("/");
    for (i, seg) in segs.iter().enumerate() {
        if i > 0 {
            acc.push('/');
        }
        acc.push_str(seg);
        if client.list_directory(&acc).await.is_ok() {
            continue;
        }
        client.create_directory(&acc, None).await?;
    }
    Ok(())
}

async fn import_fs_subtree(
    client: &Client,
    server_prefix: &str,
    fs_dir: &Path,
    rel_label: &str,
) -> Result<bool, BoxErr> {
    let mut any_error = false;
    for entry in std::fs::read_dir(fs_dir).map_err(|e| format!("import: {e}"))? {
        let entry = entry.map_err(|e| format!("import: {e}"))?;
        let path = entry.path();
        let fname = entry.file_name().to_string_lossy().to_string();
        let label = if rel_label.is_empty() {
            fname.clone()
        } else {
            format!("{rel_label}/{fname}")
        };
        if path.is_dir() {
            if let Err(reason) = validate_entity_name(&fname) {
                println!("{label} - SKIPPED: {reason}");
                continue;
            }
            let tb_sub = join_abs_dir_entry(server_prefix, &fname);
            if let Err(e) = ensure_directory_path_recursive(client, &tb_sub).await {
                println!("{label} - ERROR: {e}");
                any_error = true;
                continue;
            }
            any_error |= Box::pin(import_fs_subtree(client, &tb_sub, &path, &label)).await?;
        } else if path.is_file() {
            if let Err(reason) = validate_entity_name(&fname) {
                println!("{label} - SKIPPED: {reason}");
                continue;
            }
            let tb_path = join_abs_dir_entry(server_prefix, &fname);
            let (parent, _) = tabularium::resource_path::parent_and_final_name(&tb_path)
                .map_err(|e| -> BoxErr { e.to_string().into() })?;
            if let Err(e) = ensure_directory_path_recursive(client, &parent).await {
                println!("{label} - ERROR: {e}");
                any_error = true;
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(content) => match client.create_document(&tb_path, &content).await {
                    Ok(_) => {
                        println!("{label} - OK");
                        match fs_mtime_timestamp(&path) {
                            Ok(ts) => {
                                if let Err(e) = client.touch_document(&tb_path, Some(ts)).await {
                                    println!("{label} - ERROR: set mtime: {e}");
                                    any_error = true;
                                }
                            }
                            Err(e) => println!("{label} - WARNING: {e}"),
                        }
                    }
                    Err(e) => {
                        println!("{label} - ERROR: {e}");
                        any_error = true;
                    }
                },
                Err(e) => {
                    println!("{label} - ERROR: {e}");
                    any_error = true;
                }
            }
        }
    }
    match fs_mtime_timestamp(fs_dir) {
        Ok(ts) => {
            if let Err(e) = client.touch_document(server_prefix, Some(ts)).await {
                let label = if rel_label.is_empty() {
                    server_prefix.to_string()
                } else {
                    rel_label.to_string()
                };
                println!("{label} - ERROR: set directory mtime: {e}");
                any_error = true;
            }
        }
        Err(e) => {
            let label = if rel_label.is_empty() {
                server_prefix.to_string()
            } else {
                rel_label.to_string()
            };
            println!("{label} - WARNING: {e}");
        }
    }
    Ok(any_error)
}

#[allow(clippy::too_many_lines)]
async fn cmd_import(
    client: &Client,
    directory: &str,
    dir: &Path,
    rename_to: Option<&str>,
) -> Result<(), BoxErr> {
    if let Some(n) = rename_to {
        let n = n.trim();
        if n.is_empty() {
            return Err("import: --name must not be empty".into());
        }
        if !dir.is_file() {
            return Err("import: --name is only valid when importing a single file".into());
        }
    }
    if dir.is_file() {
        let fname = if let Some(n) = rename_to {
            n.trim().to_string()
        } else {
            dir.file_name()
                .ok_or_else(|| -> BoxErr { "import: cannot determine filename".into() })?
                .to_string_lossy()
                .to_string()
        };
        validate_entity_name(&fname).map_err(|e| -> BoxErr { format!("import: {e}").into() })?;
        let content = std::fs::read_to_string(dir)
            .map_err(|e| -> BoxErr { format!("import: {e}").into() })?;
        if directory == "/" {
            let tb_path = join_abs_dir_entry("/", &fname);
            client.create_document(&tb_path, &content).await?;
            if let Ok(ts) = fs_mtime_timestamp(dir) {
                client.touch_document(&tb_path, Some(ts)).await?;
            }
            println!("{fname} - OK");
            return Ok(());
        }
        let base = normalize_user_path(directory.trim())
            .map_err(|e| -> BoxErr { e.to_string().into() })?;
        ensure_directory_path_recursive(client, &base).await?;
        let tb_path = join_abs_dir_entry(&base, &fname);
        client.create_document(&tb_path, &content).await?;
        if let Ok(ts) = fs_mtime_timestamp(dir) {
            client.touch_document(&tb_path, Some(ts)).await?;
        }
        println!("{fname} - OK");
        return Ok(());
    }
    if !dir.is_dir() {
        return Err(format!("import: not a directory: {}", dir.display()).into());
    }
    let mut any_error = false;

    if directory == "/" {
        for entry in std::fs::read_dir(dir).map_err(|e| format!("import: {e}"))? {
            let entry = entry.map_err(|e| format!("import: {e}"))?;
            let path = entry.path();
            let label = path.display().to_string();
            if !path.is_dir() {
                println!(
                    "{label} - SKIPPED: import `/` expects only child directories at the source root"
                );
                continue;
            }
            let top_name = entry.file_name().to_string_lossy().to_string();
            if let Err(reason) = validate_entity_name(&top_name) {
                println!("{label} - SKIPPED: {reason}");
                continue;
            }
            let server_base =
                normalize_user_path(&top_name).map_err(|e| -> BoxErr { e.to_string().into() })?;
            if let Err(e) = ensure_directory_path_recursive(client, &server_base).await {
                println!("{label} - ERROR: {e}");
                any_error = true;
                continue;
            }
            any_error |= import_fs_subtree(client, &server_base, &path, "").await?;
        }
    } else {
        let base = normalize_user_path(directory.trim())
            .map_err(|e| -> BoxErr { e.to_string().into() })?;
        ensure_directory_path_recursive(client, &base).await?;
        any_error = import_fs_subtree(client, &base, dir, "").await?;
    }

    if any_error {
        return Err("import completed with one or more errors".into());
    }
    Ok(())
}

async fn export_one_file(client: &Client, tb_path: &str, out_path: &Path) -> Result<bool, BoxErr> {
    let mut err = false;
    if out_path.exists() {
        println!("{} - ERROR: destination already exists", out_path.display());
        return Ok(true);
    }
    match client.get_document(tb_path).await {
        Ok(body) => match std::fs::write(out_path, body.content().as_bytes()) {
            Ok(()) => {
                println!("{} - OK", out_path.display());
                match server_entry_modified_at(client, tb_path).await {
                    Ok(ts) => {
                        if let Err(e) = apply_fs_mtime_from_timestamp(out_path, ts) {
                            println!("{} - ERROR: {e}", out_path.display());
                            err = true;
                        }
                    }
                    Err(e) => println!("{tb_path} - WARNING: mtime: {e}"),
                }
            }
            Err(e) => {
                println!("{} - ERROR: {e}", out_path.display());
                err = true;
            }
        },
        Err(e) => {
            println!("{tb_path} - ERROR: {e}");
            err = true;
        }
    }
    Ok(err)
}

async fn export_subtree(
    client: &Client,
    server_dir: &str,
    dest_dir: &Path,
) -> Result<bool, BoxErr> {
    std::fs::create_dir_all(dest_dir).map_err(|e| format!("export: {e}"))?;
    let mut any_error = false;
    for e in client.list_directory(server_dir).await? {
        let full = join_abs_dir_entry(server_dir, e.name());
        if e.is_directory() {
            if let Err(reason) = validate_fs_name(e.name()) {
                println!("{full} - ERROR: {reason}");
                any_error = true;
                continue;
            }
            let sub = dest_dir.join(e.name());
            if let Err(err) = std::fs::create_dir_all(&sub) {
                println!("{} - ERROR: {err}", sub.display());
                any_error = true;
                continue;
            }
            any_error |= Box::pin(export_subtree(client, &full, &sub)).await?;
        } else if e.is_file() {
            if let Err(reason) = validate_fs_name(e.name()) {
                println!("{full} - ERROR: {reason}");
                any_error = true;
                continue;
            }
            let out = dest_dir.join(e.name());
            any_error |= export_one_file(client, &full, &out).await?;
        }
    }
    match server_entry_modified_at(client, server_dir).await {
        Ok(ts) => {
            if let Err(e) = apply_fs_mtime_from_timestamp(dest_dir, ts) {
                println!("{} - ERROR: {e}", dest_dir.display());
                any_error = true;
            }
        }
        Err(e) => println!("{server_dir} - WARNING: mtime: {e}"),
    }
    Ok(any_error)
}

async fn cmd_export(client: &Client, directory: &str, dest: &Path) -> Result<(), BoxErr> {
    std::fs::create_dir_all(dest).map_err(|e| format!("export: {e}"))?;
    let mut any_error = false;

    if directory.trim() == "/" {
        let entries = client.list_directory("/").await?;
        for e in entries {
            let spath = join_abs_dir_entry("/", e.name());
            if e.is_directory() {
                if let Err(reason) = validate_fs_name(e.name()) {
                    println!("{} - ERROR: {reason}", e.name());
                    any_error = true;
                    continue;
                }
                let sub = dest.join(e.name());
                if let Err(err) = std::fs::create_dir_all(&sub) {
                    println!("{} - ERROR: {err}", sub.display());
                    any_error = true;
                    continue;
                }
                any_error |= export_subtree(client, &spath, &sub).await?;
            } else if e.is_file() {
                if let Err(reason) = validate_fs_name(e.name()) {
                    println!("{} - ERROR: {reason}", e.name());
                    any_error = true;
                    continue;
                }
                let out = dest.join(e.name());
                any_error |= export_one_file(client, &spath, &out).await?;
            }
        }
    } else {
        let base = normalize_user_path(directory.trim())
            .map_err(|e| -> BoxErr { e.to_string().into() })?;
        if let Some(entries) = expand_final_segment_glob(client, &base).await? {
            for (full, row) in entries {
                if row.is_file() {
                    if let Err(reason) = validate_fs_name(row.name()) {
                        println!("{full} - ERROR: {reason}");
                        any_error = true;
                        continue;
                    }
                    let out = dest.join(row.name());
                    any_error |= export_one_file(client, &full, &out).await?;
                } else {
                    if let Err(reason) = validate_fs_name(row.name()) {
                        println!("{full} - ERROR: {reason}");
                        any_error = true;
                        continue;
                    }
                    let sub = dest.join(row.name());
                    if let Err(err) = std::fs::create_dir_all(&sub) {
                        println!("{} - ERROR: {err}", sub.display());
                        any_error = true;
                        continue;
                    }
                    any_error |= export_subtree(client, &full, &sub).await?;
                }
            }
        } else {
            any_error = export_subtree(client, &base, dest).await?;
        }
    }

    if any_error {
        return Err("export completed with one or more errors".into());
    }
    Ok(())
}

pub(crate) fn ts_rfc3339(ts: Timestamp) -> String {
    ts.try_into_datetime_local().map_or_else(
        |_| "1970-01-01T00:00:00+00:00".to_string(),
        |dt| dt.to_rfc3339(),
    )
}

fn visible_cell_len(s: &str) -> usize {
    let mut len = 0usize;
    let mut rest = s;
    while !rest.is_empty() {
        if let Some(i) = rest.find('\x1b') {
            len += rest[..i].len();
            rest = &rest[i + 1..];
            if let Some(j) = rest.find('m') {
                rest = &rest[j + 1..];
            } else {
                break;
            }
        } else {
            len += rest.len();
            break;
        }
    }
    len
}

fn pad_cell_visible(cell: &str, width: usize) -> String {
    let v = visible_cell_len(cell);
    if v >= width {
        cell.to_string()
    } else {
        format!("{cell}{}", " ".repeat(width - v))
    }
}

/// Format uptime nanoseconds as `Xd Xh Xm Xs` (same shape as archivion CLI).
fn format_uptime_ns(ns: u64) -> String {
    let total_secs = ns / 1_000_000_000;
    let days = total_secs / 86_400;
    let rem = total_secs % 86_400;
    let hours = rem / 3600;
    let rem = rem % 3600;
    let mins = rem / 60;
    let secs = rem % 60;
    format!("{days}d {hours}h {mins}m {secs}s")
}

/// Name/value lines: no header; TTY paints keys blue (not bold), values default.
fn print_cli_kv_rows(rows: &[(&str, &str)]) {
    if rows.is_empty() {
        return;
    }
    if !io::stdout().is_terminal() || std::env::var_os("NO_COLOR").is_some() {
        for (k, v) in rows {
            println!("{k}\t{v}");
        }
        return;
    }
    let wk = rows
        .iter()
        .map(|(k, _)| visible_cell_len(k))
        .max()
        .unwrap_or(0);
    for (k, v) in rows {
        let pk = pad_cell_visible(k, wk);
        println!("\x1b[34m{pk}\x1b[0m {v}");
    }
}

fn tty_style_dir_name(name: &str, is_directory: bool) -> String {
    if !is_directory || std::env::var_os("NO_COLOR").is_some() || !io::stdout().is_terminal() {
        return name.to_string();
    }
    format!("\x1b[1;34m{name}\x1b[0m")
}

/// Plain ASCII table: header row, rule line, data rows. Optional blue header when TTY.
pub(crate) fn print_cli_table(colored_header: bool, headers: &[&str], rows: &[Vec<String>]) {
    let n = headers.len();
    let mut w = vec![0usize; n];
    for (i, h) in headers.iter().enumerate() {
        w[i] = w[i].max(h.len());
    }
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(n) {
            w[i] = w[i].max(visible_cell_len(cell));
        }
    }
    let header_line = headers
        .iter()
        .enumerate()
        .map(|(i, h)| format!("{:width$}", h, width = w[i]))
        .collect::<Vec<_>>()
        .join(" ");
    if colored_header && io::stdout().is_terminal() {
        println!("\x1b[34m{header_line}\x1b[0m");
    } else {
        println!("{header_line}");
    }
    println!(
        "{}",
        w.iter()
            .map(|x| "-".repeat(*x))
            .collect::<Vec<_>>()
            .join(" ")
    );
    for row in rows {
        println!(
            "{}",
            row.iter()
                .enumerate()
                .take(n)
                .map(|(i, c)| pad_cell_visible(c, w[i]))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
}

/// Full-body edit for chat `/d` (and similar): optional conflict check on `modified_at`.
pub(crate) async fn chat_edit_full_document(client: &Client, doc_path: &str) -> Result<(), BoxErr> {
    let path =
        normalize_user_path(doc_path.trim()).map_err(|e| -> BoxErr { e.to_string().into() })?;
    let original_body = client.get_document(&path).await?;
    let original = original_body.content().to_string();
    let before_mod = original_body.modified_at();
    let doc_name = path.split_once('/').map_or(path.as_str(), |(_, d)| d);
    let ext = Path::new(doc_name)
        .extension()
        .and_then(|e| e.to_str())
        .map_or_else(|| ".md".into(), |e| format!(".{e}"));
    let mut tmp = tempfile::Builder::new()
        .prefix("tb-chat-doc-")
        .suffix(&ext)
        .tempfile()?;
    tmp.write_all(original.as_bytes())?;
    tmp.flush()?;
    let tmp_path = tmp.path().to_path_buf();
    run_editor(&tmp_path)?;
    let edited = std::fs::read_to_string(&tmp_path)?;
    if edited == original {
        return Ok(());
    }
    let check = client.get_document(&path).await?;
    if check.modified_at() != before_mod {
        return Err("document changed on the server while editing; re-open chat and retry".into());
    }
    client.replace_document(&path, &edited).await?;
    Ok(())
}

async fn cmd_edit(client: &Client, path: &str) -> Result<(), BoxErr> {
    let original = match client.get_document(path).await {
        Ok(body) => body.content().to_string(),
        Err(e) if edit_missing_document_error(&e) => {
            client.append_document(path, "").await?;
            client.get_document(path).await?.content().to_string()
        }
        Err(e) => return Err(e.into()),
    };
    let doc_name = path.split_once('/').map_or(path, |(_, d)| d);
    let ext = Path::new(doc_name)
        .extension()
        .and_then(|e| e.to_str())
        .map_or_else(|| ".md".into(), |e| format!(".{e}"));
    let mut tmp = tempfile::Builder::new()
        .prefix("tb-edit-")
        .suffix(&ext)
        .tempfile()?;
    tmp.write_all(original.as_bytes())?;
    tmp.flush()?;
    let tmp_path = tmp.path().to_path_buf();
    loop {
        run_editor(&tmp_path)?;
        let edited = std::fs::read_to_string(&tmp_path)?;
        if edited == original {
            break;
        }
        let upload = client.replace_document(path, &edited).await;
        match upload {
            Ok(()) => break,
            Err(e) => {
                eprintln!(
                    "\x1b[1;31mUpload failed: {} (temp file: {})\x1b[0m",
                    e,
                    tmp_path.display()
                );
                eprintln!("Press Enter to re-open the editor and save elsewhere if needed…");
                let mut buf = String::new();
                let _ = io::stdin().read_line(&mut buf);
            }
        }
    }
    Ok(())
}

fn edit_missing_document_error(err: &Error) -> bool {
    match err {
        Error::NotFound(_) => true,
        Error::InvalidInput(message) => message.starts_with("not found: "),
        _ => false,
    }
}

pub(crate) fn run_editor(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let ed = std::env::var("EDITOR").unwrap_or_else(|_| "vi".into());
        let p = path.to_string_lossy().replace('\'', "'\\''");
        let status = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("{ed} '{p}'"))
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other("editor exited with non-zero status"))
        }
    }
    #[cfg(not(unix))]
    {
        let ed = std::env::var("EDITOR").unwrap_or_else(|_| "notepad".into());
        let mut parts = ed.split_whitespace();
        let cmd = parts.next().unwrap_or("notepad");
        let mut c = std::process::Command::new(cmd);
        for a in parts {
            c.arg(a);
        }
        let status = c.arg(path).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other("editor exited with non-zero status"))
        }
    }
}

fn read_input(file: Option<&str>) -> Result<String, io::Error> {
    if let Some(path) = file {
        std::fs::read_to_string(path)
    } else {
        let mut s = String::new();
        io::stdin().read_to_string(&mut s)?;
        Ok(s)
    }
}

#[cfg(test)]
mod ls_sort_tests {
    use std::time::Duration;

    use tabularium::EntryId;
    use tabularium::Timestamp;
    use tabularium::rpc::ListedEntryRow;

    use super::sort_ls_rows;

    fn row(name: &str, kind: i64, mod_secs: u64) -> ListedEntryRow {
        let z = Timestamp::from(Duration::from_secs(0));
        ListedEntryRow {
            id: EntryId::from(0_i64),
            kind,
            name: name.to_string(),
            description: None,
            created_at: z,
            modified_at: Timestamp::from(Duration::from_secs(mod_secs)),
            accessed_at: z,
            size_bytes: None,
            recursive_file_count: 0,
        }
    }

    #[test]
    fn ls_sort_default_preserves_fetch_order() {
        let mut v = vec![row("b", 1, 10), row("a", 0, 5)];
        let want = v.clone();
        sort_ls_rows(&mut v, false, false);
        assert_eq!(v[0].name, want[0].name);
        assert_eq!(v[1].name, want[1].name);
    }

    #[test]
    fn ls_sort_time_newest_first_then_name_tiebreak() {
        let mut v = vec![row("b", 1, 10), row("a", 1, 10), row("c", 1, 5)];
        sort_ls_rows(&mut v, true, false);
        assert_eq!(v[0].name, "a");
        assert_eq!(v[1].name, "b");
        assert_eq!(v[2].name, "c");
    }

    #[test]
    fn ls_sort_time_newest_first_pair() {
        let mut v = vec![row("a", 1, 1), row("b", 1, 3)];
        sort_ls_rows(&mut v, true, false);
        assert_eq!(v[0].name, "b");
        assert_eq!(v[1].name, "a");
    }

    #[test]
    fn ls_sort_time_oldest_first_with_reverse_flag() {
        let mut v = vec![row("a", 1, 1), row("b", 1, 3)];
        sort_ls_rows(&mut v, true, true);
        assert_eq!(v[0].name, "a");
        assert_eq!(v[1].name, "b");
    }

    #[test]
    fn ls_sort_reverse_name_flat() {
        let mut v = vec![row("a", 0, 1), row("m", 1, 99), row("b", 0, 2)];
        sort_ls_rows(&mut v, false, true);
        assert_eq!(v[0].name, "m");
        assert_eq!(v[1].name, "b");
        assert_eq!(v[2].name, "a");
    }

    #[test]
    fn ls_sort_time_newest_first_flattens_dir_first_grouping() {
        let mut v = vec![row("dir", 0, 100), row("file", 1, 1)];
        sort_ls_rows(&mut v, true, false);
        assert_eq!(v[0].name, "dir");
        assert_eq!(v[1].name, "file");
    }
}

#[cfg(test)]
mod rm_path_tests {
    use super::rm_absolute_single_segment;

    #[test]
    fn absolute_directory_slash_form() {
        assert_eq!(rm_absolute_single_segment("/xxx").as_deref(), Some("/xxx"));
        assert_eq!(rm_absolute_single_segment("/xxx/").as_deref(), Some("/xxx"));
    }

    #[test]
    fn absolute_directory_rejects_multi_segment() {
        assert_eq!(rm_absolute_single_segment("/a/b"), None);
        assert_eq!(rm_absolute_single_segment("/"), None);
    }

    #[test]
    fn absolute_directory_requires_leading_slash() {
        assert_eq!(rm_absolute_single_segment("xxx"), None);
    }
}

#[cfg(test)]
mod chat_submit_tests {
    use super::{ChatSubmitAction, parse_chat_submit};

    #[test]
    fn slash_exit_commands_quit_chat() {
        assert!(matches!(
            parse_chat_submit("/q"),
            Ok(ChatSubmitAction::Exit)
        ));
        assert!(matches!(
            parse_chat_submit("/quit"),
            Ok(ChatSubmitAction::Exit)
        ));
        assert!(matches!(
            parse_chat_submit(" /exit "),
            Ok(ChatSubmitAction::Exit)
        ));
    }

    #[test]
    fn slash_edit_opens_editor_action() {
        assert!(matches!(
            parse_chat_submit("/e"),
            Ok(ChatSubmitAction::Edit)
        ));
        assert!(matches!(
            parse_chat_submit("/edit"),
            Ok(ChatSubmitAction::Edit)
        ));
    }

    #[test]
    fn slash_d_full_document_edit() {
        assert!(matches!(
            parse_chat_submit("/d"),
            Ok(ChatSubmitAction::EditFullDocument { path: None })
        ));
        match parse_chat_submit("/d other/doc") {
            Ok(ChatSubmitAction::EditFullDocument { path: Some(p) }) => {
                assert_eq!(p, "other/doc");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn slash_doc_alias_matches_slash_d() {
        assert!(matches!(
            parse_chat_submit("/doc"),
            Ok(ChatSubmitAction::EditFullDocument { path: None })
        ));
        match parse_chat_submit("/doc other/doc") {
            Ok(ChatSubmitAction::EditFullDocument { path: Some(p) }) => {
                assert_eq!(p, "other/doc");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn slash_history_opens_pager_action() {
        assert!(matches!(
            parse_chat_submit("/h"),
            Ok(ChatSubmitAction::History)
        ));
        assert!(matches!(
            parse_chat_submit("/history"),
            Ok(ChatSubmitAction::History)
        ));
    }

    #[test]
    fn slash_nick_changes_speaker() {
        match parse_chat_submit("/nick gigabito the great") {
            Ok(ChatSubmitAction::ChangeNick(next)) => assert_eq!(next, "gigabito the great"),
            other => panic!("unexpected parse result: {other:?}"),
        }
    }

    #[test]
    fn unknown_or_invalid_slash_commands_error() {
        assert!(parse_chat_submit("/nick").is_err());
        assert!(parse_chat_submit("/wat").is_err());
        assert!(parse_chat_submit("/q now").is_err());
        assert!(parse_chat_submit("/history all").is_err());
    }

    #[test]
    fn plain_messages_are_sent_verbatim_except_trailing_newlines() {
        match parse_chat_submit("hello there\n") {
            Ok(ChatSubmitAction::Send(message)) => assert_eq!(message, "hello there"),
            other => panic!("unexpected parse result: {other:?}"),
        }
    }
}

#[cfg(test)]
mod edit_tests {
    use tabularium::Error;

    use super::edit_missing_document_error;

    #[test]
    fn edit_missing_document_error_accepts_typed_not_found() {
        assert!(edit_missing_document_error(&Error::NotFound(
            "/kb/make-deb-howto".into()
        )));
    }

    #[test]
    fn edit_missing_document_error_accepts_rpc_stringified_not_found() {
        assert!(edit_missing_document_error(&Error::InvalidInput(
            "not found: /kb/make-deb-howto".into()
        )));
    }

    #[test]
    fn edit_missing_document_error_rejects_other_invalid_input() {
        assert!(!edit_missing_document_error(&Error::InvalidInput(
            "missing path".into()
        )));
    }
}

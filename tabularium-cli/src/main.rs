//! `tb` — thin RPC invoker for the Tabularium forge.

mod chat_markdown;
mod chat_tui;
mod execute;
mod render;
mod shell;
mod shell_path;

use std::time::Duration;

use clap::error::ErrorKind;
use clap::{Parser, Subcommand};
use tabularium::rpc::Client;
use tabularium::{HeaderMap, TailMode, merge_header_line, merge_into, parse_tb_headers_env};

fn parse_cli_tail_mode(s: &str) -> Result<TailMode, clap::Error> {
    TailMode::parse_cli_token(s).map_err(|msg| clap::Error::raw(ErrorKind::ValueValidation, msg))
}

fn merged_extra_headers(cli: &Cli) -> Result<HeaderMap, String> {
    let mut map = HeaderMap::new();
    if let Ok(raw) = std::env::var("TB_HEADERS") {
        merge_into(
            &mut map,
            parse_tb_headers_env(&raw).map_err(|e| e.to_string())?,
        );
    }
    for line in &cli.header {
        merge_header_line(&mut map, line).map_err(|e| e.to_string())?;
    }
    Ok(map)
}

#[derive(Parser)]
#[command(
    name = "tb",
    version,
    about = "tabularium CLI (JSON-RPC)",
    after_long_help = "Environment:\n  TB_HEADERS  Optional extra HTTP headers on every RPC and WebSocket request.\n              One `Name: value` per line (newline-separated); `#` starts a comment line.\n              Precedence: TB_HEADERS < repeated --header (later wins per header name).\n"
)]
pub(crate) struct Cli {
    #[arg(short = 't', default_value_t = 5)]
    timeout_sec: u64,
    #[arg(short = 'u', default_value = "http://127.0.0.1:3050")]
    api_uri: String,
    /// Extra HTTP header on every RPC and WebSocket request (repeatable). Visible in `ps` and shell history — prefer TB_HEADERS for secrets.
    #[arg(long = "header", value_name = "NAME: VALUE", action = clap::ArgAction::Append)]
    header: Vec<String>,
    #[command(subcommand)]
    command: Command,
}

/// Parse a single shell line (subcommand only; no global `-u`/`-t`).
#[derive(Parser)]
#[command(name = "tb", about = "tabularium CLI (JSON-RPC)")]
pub(crate) struct ShellCommandOnly {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Clone, Subcommand)]
pub(crate) enum Command {
    /// Append to a document (`DIR/FILE`).
    Append { path: String, file: Option<String> },
    /// Print document body (`DIR/FILE`).
    Cat {
        /// `directory/file` (absolute path).
        path: String,
        /// Skip markdown rendering when stdout is a TTY.
        #[arg(long)]
        raw: bool,
    },
    /// Follow a document over WebSocket and append chat lines (`say` on the server).
    Chat {
        #[arg(short = 'i', long = "id")]
        nickname: Option<String>,
        path: String,
        /// Skip markdown rendering in the TUI transcript pane and in `/history` when stdout is a TTY.
        #[arg(long)]
        raw: bool,
    },
    /// Copy a file or directory (`cp SRC DST`; `-r` required for directories).
    #[command(name = "cp")]
    Cp {
        /// Recursive copy (required for directory sources).
        #[arg(short = 'r', long)]
        recursive: bool,
        src: String,
        dst: String,
    },
    /// Show or set the description on a file or directory (`tb desc PATH` prints; `tb desc PATH text…` sets; `tb desc PATH ""` clears).
    Desc {
        path: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        description: Vec<String>,
    },
    /// Edit in `$EDITOR`, then open `chat` on the same path (new paths: no server write until the buffer differs from empty).
    Ec { path: String },
    /// Edit document in `$EDITOR` (`DIR/FILE`).
    Edit { path: String },
    /// Export documents to the filesystem (recursive subtree).
    Export {
        /// `/` for the whole tree, or one directory path to export.
        directory: String,
        /// Output directory (default: current directory).
        #[arg(short = 'd', long)]
        destination: Option<std::path::PathBuf>,
    },
    /// Locate files/dirs by path/name (substring, case-insensitive); tree walk via `list_directory` (not full-text — use `search`; per-file lines — use `grep`).
    Find {
        /// Limit to one directory subtree, or `/` for the whole tree (`-d /` lists all files when no name given).
        #[arg(short = 'd', long = "directory")]
        directory: Option<String>,
        /// Name substring (optional with `-d` / `--directory` = list every file under that subtree).
        name: Option<String>,
    },
    /// Regex line matches within one document (`grep` RPC; not repository-wide — use `search`).
    Grep {
        pattern: String,
        path: String,
        /// 0 = unlimited matches.
        #[arg(short = 'm', default_value_t = 0)]
        max_matches: u64,
        #[arg(short = 'v', long = "invert-match")]
        invert_match: bool,
    },
    /// First N lines of a document (`-n 0` = empty).
    Head {
        path: String,
        #[arg(short = 'n', default_value_t = 10)]
        lines: u32,
        /// Skip markdown rendering when stdout is a TTY.
        #[arg(long)]
        raw: bool,
    },
    /// Import from a local file or directory tree into the server (`/` = one top-level folder per child directory when the source is a directory, recursive).
    Import {
        /// `/` or target directory path on the server.
        directory: String,
        /// Local file or directory to read from.
        dir: String,
        /// When the source is a single file, use this document name on the server instead of the local basename.
        #[arg(short = 'n', long = "name")]
        name: Option<String>,
    },
    /// Print document body through `$PAGER` (default `less` when unset); raw if stdout is not a TTY.
    Less {
        /// `directory/file`.
        path: String,
    },
    /// List entries under a directory (files and subdirectories); omit path to list `/`.
    #[command(visible_alias = "l", visible_alias = "ll")]
    Ls {
        /// Sort by modified time (newest first; combine with `-r` for oldest first, GNU `ls -tr` style).
        #[arg(short = 't', long = "time")]
        sort_by_time: bool,
        /// Reverse sort (`-t`: oldest first; without `-t`, reverse name order).
        #[arg(short = 'r', long = "reverse")]
        reverse: bool,
        /// Directory path (omit to list repository root).
        directory: Option<String>,
    },
    /// List entries by modified time (newest first); shorthand for `ls -t`.
    Lt {
        /// Oldest first (`ls -tr` style).
        #[arg(short = 'r', long = "reverse")]
        reverse: bool,
        /// Directory path (omit to list repository root).
        directory: Option<String>,
    },
    /// Create a directory (absolute or relative to root).
    #[command(name = "mkdir")]
    Mkdir {
        /// Create parent directories as needed (POSIX `mkdir -p`).
        #[arg(short = 'p', long)]
        parents: bool,
        #[arg(long)]
        description: Option<String>,
        path: String,
    },
    /// Move or rename a directory or file (`mv SRC DST`).
    Mv { src: String, dst: String },
    /// Create or replace a document (`DIR/FILE`).
    Put {
        /// `directory/file`.
        path: String,
        /// Read from file instead of stdin.
        file: Option<String>,
    },
    /// Rebuild Tantivy: `/` = full rebuild, or one directory path for scoped reindex.
    Reindex {
        /// `/` or directory path.
        path: String,
    },
    /// Remove file(s) (`DIR/FILE` or `DIR/GLOB`) or director(ies) (`DIR` or glob); `-r` only for directory paths.
    Rm {
        #[arg(short = 'r', long)]
        recursive: bool,
        path: String,
    },
    /// Indexed full-text search across document body, file name, and description (`search` RPC; path substring walk — use `find`; line-regex — use `grep`).
    Search {
        /// Limit hits to this directory subtree.
        #[arg(short = 'd', long = "directory")]
        directory: Option<String>,
        /// Query string.
        query: Vec<String>,
    },
    /// Enter interactive shell (readline, history, completion).
    Shell {
        /// Create missing directories (POSIX `mkdir -p`); requires initial CWD.
        #[arg(short = 'p', long = "parents", requires = "cwd")]
        parents: bool,
        /// Start in this directory (same path rules as `cd`; omit for repository root).
        cwd: Option<String>,
    },
    /// Inclusive 1-based line slice.
    Slice {
        path: String,
        start: u32,
        end: u32,
        /// Skip markdown rendering when stdout is a TTY.
        #[arg(long)]
        raw: bool,
    },
    /// Metadata and line count.
    Stat { path: String },
    /// Server diagnostics (`test` RPC): product name, version, uptime (nanoseconds).
    Test,
    /// Last N lines of a document (`-n +K` = from line K, GNU `tail`; `-n 0` = no lines; with `-f`, no initial output then follow).
    Tail {
        #[arg(short = 'n', default_value = "10", value_parser = parse_cli_tail_mode)]
        tail: TailMode,
        /// Follow via WebSocket (`ws://` / `wss://`); same host as `-u`.
        #[arg(short = 'f', long)]
        follow: bool,
        path: String,
        /// Skip markdown rendering when stdout is a TTY.
        #[arg(long)]
        raw: bool,
    },
    /// `PATH` creates/bumps `modified_at`; optional second arg sets exact `modified_at` (Unix **seconds** if all digits, else date/time string).
    Touch { path: String, time: Option<String> },
    /// Block until `DIR/FILE` changes (server long-poll timeout applies).
    Wait { path: String },
    /// Byte/line/word/char counts.
    Wc { path: String },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();
    let extra = merged_extra_headers(&cli)?;
    let client = Client::init_with_extra_headers(
        &cli.api_uri,
        Duration::from_secs(cli.timeout_sec.max(1)),
        extra,
    )?;

    match cli.command {
        Command::Shell { parents, cwd } => {
            shell::run_shell(
                client,
                cli.api_uri,
                cli.timeout_sec,
                cwd,
                parents,
                shell::ShellSpawnContext {
                    header_flags: cli.header.clone(),
                },
            )
            .await?;
        }
        cmd => {
            let opts = execute::execute_opts_from_command(&cmd);
            match execute::execute(&client, cmd, execute::ExecuteContext::Cli, None, opts).await {
                Ok(execute::ExecuteOutcome::Ok) => {}
                Ok(execute::ExecuteOutcome::Interrupted) => std::process::exit(130),
                Err(e) => return Err(e),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod cli_parse_tests {
    use super::*;

    #[test]
    fn touch_parses_path_and_optional_time() {
        let c = Cli::try_parse_from(["tb", "-u", "http://x", "touch", "a/b"]).unwrap();
        let Command::Touch { path, time } = c.command else {
            panic!("expected touch");
        };
        assert_eq!(path, "a/b");
        assert!(time.is_none());
        let c2 =
            Cli::try_parse_from(["tb", "-u", "http://x", "touch", "a/b", "1712345678"]).unwrap();
        let Command::Touch { path, time } = c2.command else {
            panic!("expected touch");
        };
        assert_eq!(path, "a/b");
        assert_eq!(time.as_deref(), Some("1712345678"));
    }

    #[test]
    fn mkdir_parses_parents_flag() {
        let c = Cli::try_parse_from(["tb", "-u", "http://x", "mkdir", "-p", "/a/b"]).unwrap();
        let Command::Mkdir {
            parents,
            path,
            description,
        } = c.command
        else {
            panic!("expected mkdir");
        };
        assert!(parents);
        assert_eq!(path, "/a/b");
        assert!(description.is_none());
    }

    #[test]
    fn tail_parses_plus_n() {
        let c = Cli::try_parse_from(["tb", "-u", "http://x", "tail", "-n", "+2", "a/b"]).unwrap();
        let Command::Tail { tail, .. } = c.command else {
            panic!("expected tail");
        };
        assert_eq!(tail, TailMode::FromLine(2));
    }

    #[test]
    fn tail_rejects_plus_zero() {
        let Err(err) = Cli::try_parse_from(["tb", "-u", "http://x", "tail", "-n", "+0", "a/b"])
        else {
            panic!("expected parse failure for +0");
        };
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn ls_parses_time_and_reverse() {
        let c = Cli::try_parse_from(["tb", "-u", "http://x", "ls", "-tr"]).unwrap();
        let Command::Ls {
            sort_by_time,
            reverse,
            directory,
        } = c.command
        else {
            panic!("expected ls");
        };
        assert!(sort_by_time);
        assert!(reverse);
        assert!(directory.is_none());
    }

    #[test]
    fn ls_sort_flags_with_directory() {
        let c = Cli::try_parse_from(["tb", "-u", "http://x", "ls", "-t", "notes"]).unwrap();
        let Command::Ls {
            sort_by_time,
            reverse,
            directory,
        } = c.command
        else {
            panic!("expected ls");
        };
        assert!(sort_by_time);
        assert!(!reverse);
        assert_eq!(directory.as_deref(), Some("notes"));
    }

    #[test]
    fn import_parses_rename_name_flag() {
        let c = Cli::try_parse_from([
            "tb",
            "-u",
            "http://x",
            "import",
            "/notes",
            "local.txt",
            "-n",
            "server.md",
        ])
        .unwrap();
        let Command::Import {
            directory,
            dir,
            name,
        } = c.command
        else {
            panic!("expected import");
        };
        assert_eq!(directory, "/notes");
        assert_eq!(dir, "local.txt");
        assert_eq!(name.as_deref(), Some("server.md"));
    }

    #[test]
    fn lt_parses_like_ls_time_sorted() {
        let c = Cli::try_parse_from(["tb", "-u", "http://x", "lt", "-r", "notes"]).unwrap();
        let Command::Lt { reverse, directory } = c.command else {
            panic!("expected lt");
        };
        assert!(reverse);
        assert_eq!(directory.as_deref(), Some("notes"));
    }

    #[test]
    fn shell_parses_optional_initial_cwd() {
        let c = Cli::try_parse_from(["tb", "-u", "http://x", "shell", "notes"]).unwrap();
        let Command::Shell { parents, cwd } = c.command else {
            panic!("expected shell");
        };
        assert!(!parents);
        assert_eq!(cwd.as_deref(), Some("notes"));
        let c2 = Cli::try_parse_from(["tb", "-u", "http://x", "shell"]).unwrap();
        let Command::Shell { parents, cwd } = c2.command else {
            panic!("expected shell");
        };
        assert!(!parents);
        assert!(cwd.is_none());
    }

    #[test]
    fn shell_parses_parents_with_cwd() {
        let c = Cli::try_parse_from(["tb", "-u", "http://x", "shell", "-p", "notes/sub"]).unwrap();
        let Command::Shell { parents, cwd } = c.command else {
            panic!("expected shell");
        };
        assert!(parents);
        assert_eq!(cwd.as_deref(), Some("notes/sub"));
        let c2 = Cli::try_parse_from(["tb", "-u", "http://x", "shell", "notes/sub", "-p"]).unwrap();
        let Command::Shell { parents, cwd } = c2.command else {
            panic!("expected shell");
        };
        assert!(parents);
        assert_eq!(cwd.as_deref(), Some("notes/sub"));
    }

    #[test]
    fn shell_parents_without_cwd_errors() {
        match Cli::try_parse_from(["tb", "-u", "http://x", "shell", "-p"]) {
            Ok(_) => panic!("expected parse error for shell -p without cwd"),
            Err(e) => assert_eq!(e.kind(), ErrorKind::MissingRequiredArgument),
        }
    }

    #[test]
    fn cp_parses_recursive_flag() {
        let c = Cli::try_parse_from(["tb", "-u", "http://x", "cp", "-r", "a", "b"]).unwrap();
        let Command::Cp {
            recursive,
            src,
            dst,
        } = c.command
        else {
            panic!("expected cp");
        };
        assert!(recursive);
        assert_eq!(src, "a");
        assert_eq!(dst, "b");
    }

    #[test]
    fn ec_parses_path() {
        let c = Cli::try_parse_from(["tb", "-u", "http://x", "ec", "notes/x.md"]).unwrap();
        let Command::Ec { path } = c.command else {
            panic!("expected ec");
        };
        assert_eq!(path, "notes/x.md");
    }
}

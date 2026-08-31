# feat: tb wc with POSIX flags, glob, recursive walk, and MCP support

## Summary

Extends `tb wc` to GNU/POSIX-like behaviour: `-l`/`-w`/`-c`/`-m` flags, final-segment glob, recursive directory walk (`-r`), a single JSON-RPC round-trip per invocation, and an updated MCP tool with catalog examples. Path resolution and metric filtering live in the library crate; CLI, shell, RPC, and MCP share the same semantics and response shapes.

## Motivation

- `tb wc -l /docs/*` did not behave like users expect from Linux `wc` (glob was only partially handled on the CLI read path, not in server-side `wc`).
- Need Linux-like errors (`is a directory`) and a hint to use `-r` when a glob matches directories only.
- MCP agents need structured `wc` with documented request/response examples.

## Changes

### Library (`tabularium`)

| Area | Detail |
|------|--------|
| **`glob_path.rs`** (new) | Final-segment glob, `path_is_directory`, recursive file collection, `resolve_wc_file_paths` |
| **`wc.rs`** (new) | `WcMetricsFilter`, JSON helpers, `sum_wc_stats`, unit tests |
| **`rpc/client.rs`** | `Client::wc()`, types `WcResponse` / `WcMetrics` / `WcFileEntry`; legacy `document_wc` kept |
| **`Cargo.toml`** | `globset` under feature `db` |
| **`db/meta.rs`** | `WcStats::from_totals` |

### Server (`tabularium-server`)

| Area | Detail |
|------|--------|
| **`web.rs`** | RPC `wc`: glob, `recursive`, metric flags; `{ files, total }` only when **two or more** files resolve |
| **`mcp.rs`** | `WcArg`, expanded catalog and `#[tool(description)]` with examples |
| **`res/mcp/help.txt`** | `wc` paragraph |
| **`tests/text_ops.rs`** | Glob batch, directory guard, dirs-only hint, recursive walk, single-glob flat response |

### CLI (`tabularium-cli`)

| Area | Detail |
|------|--------|
| **`main.rs`** | `Wc` flags and `-r`; cleaner error output via `print_cli_message` |
| **`execute.rs`** | Single `client.wc()` call; GNU/TTY formatting; shared `print_cli_message` |
| **`shell.rs`** | `flags_for("wc")`, parse tests, errors via `print_cli_message` |
| **`Cargo.toml`** | Removed direct `globset` dependency (via library) |

### Tests & docs

- **`tests/test_cli_wc.py`** (new) — glob, directory error, recursive glob, single-file `-l`
- **`docs/json-rpc-methods.md`** — updated `wc` params and response

### Refactors

- DRY in `wc.rs` (`wc_metrics_object`, `format_optional_counts`, `column_labels`)
- Simplified `print_wc_gnu_line` in CLI
- Batch JSON only when `paths.len() > 1` (one glob match → flat object, same as a literal path)
- Unified CLI and shell error UX (strips `invalid input:` / `not found:` prefixes)

## Behaviour

| Input | Result |
|-------|--------|
| `tb wc -l /docs/readme.md` | single number (pipe-friendly) |
| `tb wc -l '/docs/*'` | per-file lines + `total` |
| `tb wc -l /projects/bms` | error: `is a directory` + hint to use `-r` |
| `tb wc -l -r 'projects/bms/*'` | all files under each matched child directory + `total` |
| `tb wc -l -r /projects/bms` | whole subtree + `total` |
| RPC glob → one file | flat `{ lines: … }` (not batch) |
| RPC glob → two or more files | `{ files: [...], total: {...} }` |

**Defaults:** no flags → GNU columns `lines`, `words`, `bytes`. **`-c`** = UTF-8 byte length (POSIX); **`-m`** = Unicode scalar count.

## API notes

- **Backward compatible:** `document_wc()` without metric flags still returns all four fields in a flat object.
- **MCP:** `WcArg` with optional booleans; explicit `false` is not sent on the wire.
- **ACL:** glob checks `check_read(parent)` first, then per-file `check_read` (fail-fast on first denial).
- **Performance (Stage 1):** N resolved files ⇒ N full body reads via `document_wc`. Design note in Tabularium KB: `/projects/tabularium/design/wc-performance.md`.

## Files to include in the commit

```
Cargo.lock
docs/json-rpc-methods.md
tabularium/Cargo.toml
tabularium/src/glob_path.rs
tabularium/src/wc.rs
tabularium/src/lib.rs
tabularium/src/db/meta.rs
tabularium/src/rpc/client.rs
tabularium/src/rpc/mod.rs
tabularium-cli/Cargo.toml
tabularium-cli/src/main.rs
tabularium-cli/src/execute.rs
tabularium-cli/src/shell.rs
tabularium-server/src/web.rs
tabularium-server/src/mcp.rs
tabularium-server/res/mcp/help.txt
tabularium-server/tests/text_ops.rs
tests/test_cli_wc.py
```

Do **not** include local/unrelated untracked files: `config.fuse-test.toml`, `entities.json`, `mempalace.yaml`.

## Suggested commit message

```
feat: add tb wc flags, glob, recursive walk, and MCP support
```

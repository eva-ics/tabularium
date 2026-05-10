# Changelog

## 0.1.7 - 2026-05-11

Authentication, concurrency, and operator tooling release.

- Adds built-in stage-1 authentication and authorization across REST,
  JSON-RPC, WebSocket, and optional MCP auth: PSKs mapped to ACLs, `whoami`,
  admin `acl_*` / `psk_*` workflows, and filtered list/search results.
- Adds trusted upstream assertion support via `[oidc]` with local JWK/JWKS
  verification and refresh, so deployments can accept proxy-minted JWT
  assertions without treating hope as a security boundary.
- Adds file `revision` UUIDs to file metadata and compare-and-swap writes via
  `only_if_revision` on `put_document` and `create_document`, exposing
  revision tokens through file reads and metadata APIs.
- Adds `append_if_not_contains` for atomic marker-checked appends to existing
  files, useful for idempotent meeting, task, and agent update flows.
- Improves the Web UI with a PSK auth gate, trusted-session handling,
  ACL-aware stats, preview polish, and entry workflow fixes.
- Expands CLI, MCP, and docs coverage around auth, revisions, force guards,
  and trusted `mcp.full` deployments, with regression matrices for ACLs,
  stale-write detection, and guarded appends.

- **Breaking**: `put_document` and `append_document` (JSON-RPC and MCP) now take an
  optional `force` boolean (default `false`). With `force=false`, an existing target
  fails with `Duplicate` (`-32002`); pass `force=true` to deliberately overwrite
  (`put_document`) or append to (`append_document`) an existing scroll. Missing
  targets are always created. Old callers that relied on silent overwrite/append
  must now pass `force=true`. The check-then-create is atomic at the storage layer
  (SQLite `UNIQUE(parent_id, name)`), so concurrent `force=false` callers cannot
  both create.
- `say_document` is intentionally exempt from the guard (cannot create new
  documents; meeting/conversation rite only) — documented in MCP help and
  `docs/json-rpc-methods.md`.
- REST `PUT` / `PATCH /api/doc/...` keep HTTP upsert convention (effectively
  `force=true`); use the JSON-RPC surface when create-only behaviour is required.
- CLI: `tb put` and `tb append` gain `-f` / `--force` for explicit overwrite/append.

## 0.1.5 - 2026-04-11

Web UI workflow and rendering release.

- Adds Web UI entry management for creating files and folders, editing
  descriptions, simple same-parent renames, and confirmed deletes.
- Adds KaTeX-based math rendering for markdown preview and chat views, plus
  docs and tests for inline and block formulas.
- Improves print-friendly Web UI layouts, CLI chat scrolling behavior, and RPC log sanitization.

## 0.1.4 - 2026-04-08

Web UI and packaging release.

- Improves the embedded Web UI with screenshots, styling and workspace flow fixes, plus better reference handling coverage.
- Adds Homebrew installation support, server `--config`, `mkdir -p`, and related path and release-target packaging fixes.
- Docker images

## 0.1.3 - 2026-04-06

Polish release.

- Adds workspace-wide version management with `just bump` and `just set-version`, plus CI and packaging updates around the embedded web UI build.
- Improves the Web UI with dock and fullscreen preview polish, stronger search highlighting, and correct handling of internal markdown references, including reusable copied links.
- Adds valid GFM table rendering support in the Web UI and CLI chat, with regression coverage for tables and reference navigation.
- Refreshes docs with an AI agent setup guide, removes stale public terminology, and tightens name validation to reject `.` and `..`.

## 0.1.2 - 2026-04-06

First public release.

- Publishes the Tabularium workspace with the core library, `tabularium-server`, `tb` CLI, and embedded web UI.
- Exposes the document store over REST, JSON-RPC, and MCP, backed by SQLite storage and Tantivy search.
- Includes user-facing workflows for browsing, editing, searching, importing, exporting, and chat or meeting-style document updates.

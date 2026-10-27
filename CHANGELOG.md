# Changelog

## Unreleased

## 0.1.9 - 2026-10-27

### Security

- Refresh Web UI dependencies and lockfile to resolve npm audit findings.
- Upgrade KaTeX to 0.19.1 across math-rendering dependencies and use the
  patched React Router 7.18.5 for bmat as well as the application.

### Maintenance

- Move project, installation and Homebrew links to Velor; use the
  `velortech/tabularium` Docker namespace and retire the standalone website.

## 0.1.8 - 2026-10-27

### Added

- Opt-in OTLP/HTTP protobuf traces and MCP/runtime metrics, with bounded shutdown
  and RPC parameter previews excluded from exported span events ([#7](https://github.com/velortechnologies/tabularium/pull/7)).
- Mermaid diagrams in document and chat previews ([#7](https://github.com/velortechnologies/tabularium/pull/7)).
- Adds optional `[anonymous]` path ACL: when auth is enabled and credentials are
  absent, requests use a configured non-admin allow/deny principal. Present but
  invalid PSK/OIDC credentials still fail closed. Web UI opens the public view
  without a key and offers Login / Logout around PSK sessions
  ([#7](https://github.com/velortechnologies/tabularium/pull/7)).
- Web UI maps human librarium URLs (`/projects/…/doc.md`) to the entries
  browser instead of sending unknown paths to the library root; file/directory
  routing uses the server entry kind, including extensionless files and dotted
  directory names ([#6](https://github.com/velortechnologies/tabularium/pull/6)).
- Optional OpenAI-compatible embeddings with custom headers and a rebuildable
  semantic sidecar; lexical, semantic and hybrid search/reindex modes across
  REST, JSON-RPC, MCP, CLI and Web UI.
- Bounded search responses with result limits, JSON byte budgets, explicit
  omission flags, exact excerpts, source revisions and modification times.
- Exclusive UNIX-nanosecond modification bounds and deterministic relevance or
  modification-time ordering across all search interfaces.
- Revision-checked slice reads and structured CLI search output.
- Per-document JSON metadata with revision-checked REST, JSON-RPC and MCP APIs,
  `tb meta` viewing/editing, and import/export sidecars. Metadata is included in
  lexical search and preserved by body edits, copies and moves.
- Read-only metadata preview with a Meta/File toggle in the Web UI.

### Improved

- Search filters authorized candidates before top-k selection. Semantic excerpts
  remain tied to current source revisions and exact line ranges; background
  refresh handles writes, moves and deletions.
- MIME-aware JSON and shell previews, plain-text previews for `.txt` and `.list`,
  a search-kind selector, and mobile access to the copy-path button.
- Neutral interface/help wording, Selenium focus handling, and regression
  coverage for search bounds, ordering, transports and offline semantic retrieval.

### Fixed

- Restore MCP access using non-loopback Host headers after the transport SDK
  update, matching EVA MCP behavior; PSK/OIDC authentication and ACLs are unchanged.
- Makes Web UI path copying more reliable with a local clipboard helper, a
  selection-based fallback, and manual copying when browser access is denied.

### Upgrade notes

- JSON metadata adds an automatically migrated SQLite column with an empty-object
  default; existing document bodies, IDs, revisions and ACLs are preserved.
- Search enhancements do not require a document database migration. The semantic
  sidecar is rebuildable; the Tantivy schema is unchanged.

## 0.1.7 - 2026-05-11

Authentication, concurrency, and operator tooling release.

- Adds built-in stage-1 authentication and authorization across REST,
  JSON-RPC, WebSocket, and optional MCP auth: PSKs mapped to ACLs, `whoami`,
  admin `acl_*` / `psk_*` workflows, and filtered list/search results.
- Adds trusted upstream assertion support via `[oidc]` with local JWK/JWKS
  verification and refresh, so deployments can accept proxy-minted JWT
  assertions with locally verified signatures.
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
- Adds published Docker images.

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

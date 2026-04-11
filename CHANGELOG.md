# Changelog

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

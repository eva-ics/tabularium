# Agent notes (tabularium)

For machine spirits and maintainers: layout, commands, and doctrine.

**Coding standards**: the Tabularium MCP scroll `kb/coding-rules` must be applied for new code blocks when architecture or conventions are uncertain.

## Layout

- Workspace root: [Cargo.toml](Cargo.toml) — members `tabularium/`, `tabularium-server/`, `tabularium-cli/`.
- Library crate: [tabularium/](tabularium/) — Cargo features **`db`** (SQLite + Tantivy + `Database`), **`client`** (`rpc::Client` + `reqwest`; implies **`db`**), **`full`** = both. Default = **`client`** (same as full stack). Server depends on **`db` only**; CLI on **`client`**. See `just check-tabularium-db` / `just check-tabularium-core`.
- Server binary: [tabularium-server/](tabularium-server/) (`tabularium-server`), Axum REST `/api/doc/*` + `/api/search` + JSON-RPC `POST /rpc`.
- CLI binary: [tabularium-cli/](tabularium-cli/) (`tb`).
- Example config: [config.toml.example](config.toml.example) — copy to `config.toml` for a writable local path, or rely on `just run` (uses the example when `config.toml` is absent). API scrolls: [docs/json-rpc-methods.md](docs/json-rpc-methods.md), [docs/rest-api.md](docs/rest-api.md), [docs/curl-examples.md](docs/curl-examples.md).
- Optional Python checks: [requirements-dev.txt](requirements-dev.txt), [tests/](tests/) (`pytest tests/`) — set `TABULARIUM_TEST_URL` when a server is running; for CLI tests, set `TABULARIUM_TB_BIN` or install `tb` on `PATH`.
- **Selenium / Web UI (`@pytest.mark.webui`)**: do **not** run against `localhost` or `127.0.0.1`. Tests are skipped unless `TABULARIUM_TEST_URL` or `TABULARIUM_URL` points at host **`10.90.1.122`** (same port as the running server). Use the integration host for all Web UI browser tests; `just ci-test` binds the ephemeral server on `0.0.0.0:<port>` and sets the base URL to `http://10.90.1.122:<port>` automatically.
- Docs: [docs/](docs/). Meetings/tasks: tabularium MCP

## Behaviour notes (meeting consensus)

- **Index / DB drift**: if the process dies between a SQLite write and a Tantivy commit, the index can disagree with the DB. Stage 1 accepts this; a future `just`/admin rebuild hook is the intended remedy.
- **`accessed_at` / `touch`**: every successful logical read via `Database::get_document` calls `Storage::touch` after content is resolved (cache hit or miss). If `touch` fails, the error is propagated to the caller (no `Ok` with stale silence).
- **Read cache**: `moka` async cache holds document bodies only; `cache_size == 0` keeps the cache object but never stores entries. Mutations invalidate the affected document id after a successful storage write.
- **`reindex`**: `Database::reindex(None)` clears the Tantivy index and rebuilds from every `documents` row. `Database::reindex(Some(category_id))` re-upserts only rows in that directory (internal `category_id`); other indexed documents are left as-is. Public API uses directory/path vocabulary only.
- **Tracing**: the library uses `tracing` spans on `SqliteDatabase::init`, `Database` façade methods, `SqliteStorage` (`Storage`), and `SearchIndex` I/O. Install a subscriber in binaries/tests (e.g. `tracing_subscriber`) and set `RUST_LOG=tabularium=debug` (or `trace`) to see them; the library crate does not initialize global logging.

## Build & test

```bash
cargo fmt --all
just test
```

(`just test` is **release**; use `just test-dev` for debug builds.)

Clippy (Enginseer script on `PATH`): `just clippy` or invoke `clippy` directly.

Remote integration rite: **`just ci-test`** — rsync to `/opt/tabularium` on **root@10.90.1.122**, release build, ephemeral `tabularium-server` (listen `0.0.0.0:<port>`) + pytest with **`TABULARIUM_TEST_URL=http://10.90.1.122:<port>`** (required for Selenium), then **`just test`**. Override `CI_HOST`, `CI_PROJECT`, `CI_LISTEN`. See [scripts/ci-test.sh](scripts/ci-test.sh).

Local release-only (no SSH / no Python): `just test-ci`.

# REST API

Base path: `/api`. Document tree: **`/api/doc`**. `GET /api/doc` lists **root-level** entries (files and directories); deeper paths use additional URL segments (names or numeric ids).

## Authentication

When `[server].authenticate = true`, REST uses the same built-in auth rules as
the web UI, `/ws`, and `POST /rpc`:

- default credential header: `X-Auth-Key`
- optional upstream assertion header from `[oidc].header` (`X-JWT-Assertion` by
  default)
- if the assertion header is present, REST validates it and fails closed on
  error; no PSK fallback
- if the assertion header is absent, REST falls back to `X-Auth-Key`

Health probe:

- `GET /api/test` stays open even when auth is enabled

Identity probe:

- `GET /api/whoami` requires the same auth as the rest of the protected API and
  returns the resolved ACL / permission view for the caller

Failure semantics:

- `401 Unauthorized` — missing or unknown credential
- `403 Forbidden` — known credential, but ACL denies the action

Directory listings and search results are filtered before they are returned.

## Introspection

- `GET /api/test` — JSON `{ product_name, product_version, uptime, authenticate_api }`
- `GET /api/whoami` — JSON `{ acl, admin, permissions }`

## Document tree (`/api/doc`)

- `GET /api/doc` — JSON array: `id`, `kind`, `name`, `description`, timestamps, `size_bytes`, `recursive_file_count` (same shape as JSON-RPC `list_directory` at `/`).
- `POST /api/doc` — body `{ "name": "...", "description": null }` or `{ "path": "/abs/path", "description": null, "parents": false }`. Optional `parents: true` creates missing parent directories (POSIX `mkdir -p`; best-effort / non-atomic; idempotent if the leaf directory already exists; see JSON-RPC `create_directory`). `201` + `Location: /api/doc/<path>`.

Path segments under `/api/doc/…` accept **name** or **numeric id** per segment.

- `GET /api/doc/{dir}` — when `{dir}` resolves as a directory: list entries (same shape as `GET /api/doc` for that path).
- `POST /api/doc/{dir}` — JSON `{ "name": "...", "content": "..." }`, `application/x-www-form-urlencoded` (`name` & `content`), or `multipart/form-data` with those fields. `201` + `Location` to the document path.
- `GET /api/doc/{dir}/{name}` — full document JSON including `content`.
- `PUT /api/doc/{dir}/{name}` — JSON `{ "content": "..." }`, `application/x-www-form-urlencoded` (`content=...`), raw UTF-8 body, or `multipart/form-data` field `content`. Replaces body. `204`. (HTTP `PUT` is upsert by convention; the JSON-RPC `force` guard does not apply here — use `POST /rpc` `put_document` with `force=false` when you need create-only semantics.)
- `PATCH /api/doc/{dir}/{name}` — same content shapes as `PUT`; appends (with newline). `204`. (Same upsert convention; use JSON-RPC `append_document` with `force=false` for create-only behaviour.)
- `DELETE /api/doc/{dir}/{name}` — `204`.

`PATCH` on a directory-only path is not exposed (documents only).

## Search

- `GET /api/search?q=...&kind=hybrid&with_status=true`
- `POST /api/search` — JSON `{ "q": "..." }`, `application/x-www-form-urlencoded` (`q=...`), or `multipart/form-data` with field `q`.

By default the response remains a JSON hit array. `with_status=true` returns `{ hits, lexical_status, semantic_status }`. Hits also expose optional `line_end` and `revision` for semantic excerpts. Both formats carry `X-Tabularium-Lexical-Status` and `X-Tabularium-Semantic-Status` response headers.

`POST /api/reindex` accepts JSON `{ "path": "/", "kind": "semantic" }`; admin permission is required when API authentication is enabled.

Errors: JSON `{ "error": "message" }` with `400`, `401`, `403`, `404`, `409`,
or `500` as appropriate.


## Semantic and hybrid search

Search and reindex accept `kind`: `lexical`, `semantic`, or `hybrid` (default).
Optional `[semantic]` config enables OpenAI-compatible embeddings, including custom HTTP headers.
The semantic sidecar is rebuilt without migrating the document database.
See [semantic search](semantic-search.md) for configuration, API status envelopes, upgrade steps, and fallback behavior.

## Bounded search

Search accepts `limit` (1..256, default 256) and `max_output_bytes` (512..1048576, default 65536) in GET parameters or POST JSON/form. The byte budget covers compact UTF-8 search JSON, not HTTP/JSON-RPC/MCP wrappers. Whole hits are omitted to fit, never rewritten or cut mid-quote. An oversized hit can yield zero hits with truncation.

Either bound opts into `{hits, lexical_status, semantic_status, has_more, truncated}`. `with_status=true` also opts in. Without them the legacy hit array remains; REST exposes omission through `x-tabularium-search-truncated`. Omission flags are ACL/path filtered and do not promise pagination or a total count. Engine availability remains a separate status.

Hits include `path`, `snippet`, `revision`, `modified_at` (Unix nanoseconds), `start_line`/`end_line`, and backward-compatible `line_number` (match line)/`line_end` (excerpt end). Lines are inclusive and 1-based. Lexical excerpts contain at most 768 UTF-8 source bytes, and can begin/end mid-line; semantic chunks follow configuration. Search ranks by relevance with deterministic tie-breaking. Technical modification dates do not imply truth/currentness.

RPC/MCP `slice` accepts `expected_revision` and returns `{text, revision}`. Stale or concurrently changed source yields `RevisionMismatch` (-32003). No atomicity is promised between search and slice. CLI equivalents: `tb search --kind hybrid --limit 10 --max-output-bytes 16384 --json query` and `tb slice /path 1 10 --expected-revision UUID`. No schema migration or reindex is needed.

## Modification-time filters and ordering

Search accepts `sort=relevance` (default) or `sort=modified_desc`.
Equal primary keys are ordered by canonical path ascending, then document ID.
`modified_after` and `modified_before` are **exclusive unsigned UNIX
nanoseconds**, the same unit as returned `modified_at`; equality excludes the
document. Equal bounds yield an empty interval; a reversed interval is invalid.
Missing bounds are unbounded. Modification time is a technical timestamp, not
an assertion of factual currency. No source database schema change is required.

Filters and ACL/path constraints apply before per-engine result limits.
For hybrid search, relevance retains reciprocal-rank fusion; modified_desc
orders the union of each engine's newest matching candidates. Scores remain
engine/fusion scores even when ordering by modification time.
Existing array responses remain arrays unless with_status=true, limit, or
max_output_bytes requests an envelope. sort/time filters alone do not change shape.

CLI example:
```sh
tb search --kind hybrid --sort modified_desc --modified-after 1792713600000000000 --limit 10 --json memory
```

REST GET uses query parameters, REST POST accepts JSON/form/multipart, RPC/MCP
use the same snake_case keys. The Web UI provides an order selector and local
date/time inputs, converting milliseconds to nanoseconds using BigInt strings.

CI includes deterministic equal-date fixtures and offline semantic retrieval
controls (literal miss, semantic hit, unrelated negative, exact quote/slice,
revision and ACL/path). These validate retrieval plumbing, not real-model
relevance. Real embeddings still need a deployment-specific evaluation set.

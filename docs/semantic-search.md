# Hybrid lexical and semantic search

`kind` is the same lowercase enum on CLI, REST, JSON-RPC and MCP:

- `lexical`: Tantivy full-text/name/description search.
- `semantic`: embedding similarity; requires configured and initialized semantic storage.
- `hybrid` (default): both engines concurrently, reciprocal-rank fusion (`k=60`), one result per document.

Without `[semantic]`, default hybrid remains lexical-only and works with old databases. Explicit semantic mode returns an actionable error when not configured, unindexed, incompatible or unavailable. Hybrid keeps successful results from the other engine and exposes status; a failed semantic branch never causes a silent claim of full coverage.

## Storage and upgrade

- `tabularium.db` remains authoritative; its schema is not changed by this feature.
- `tabularium.index` remains the Tantivy directory.
- A separate sibling `tabularium.semantic.index/semantic.db` stores derived vectors, exact byte/line coordinates, document revisions, model/preprocessing identity and persistent dirty IDs. It is disposable and reconstructible. Treat it as private data like the source database.
- Initial upgrade: add optional `[semantic]` configuration, restart, run `tb reindex --kind semantic /` once. Plain `tb reindex` rebuilds both configured engines.
- Model ID/revision, prefixes, normalization or chunk settings changing require a full semantic rebuild, not a migration. Pin the provider model version with `model_revision`; alias-only model changes on a remote server cannot be reliably detected when dimensions stay equal.
- Moving the same inference model between CPU and GPU does not require a new client or database layout.

Rebuilds are synchronous API operations and may take minutes; the typed client's reindex call allows up to one hour. CLI is preferred for long first rebuilds; MCP/HTTP callers must configure an adequate transport timeout. A successful rebuild atomically replaces the selected semantic scope only after all embedding batches succeeded. A model/config change or first initialization requires a full rebuild, not a scoped one. Lexical and semantic rebuilds are independent; a hybrid rebuild can finish lexical and then fail semantic, which is reported as an error.

Document create/update/append/describe/rename/move/delete enqueue sidecar refresh without waiting for the model. Queues persist, coalesce by document ID with generation guards and retry after failures. Startup reconciles source IDs against the derived store after it has been initialized, recovering a crash between source commit and dirty-queue insertion. Missing/revision-mismatched source excerpts are never returned. Indexing does not touch document `accessed_at`.

## Embeddings service

The Rust server is an HTTP client, not a model runtime. `base_url` includes the provider's API prefix (usually `/v1`), and the client calls `POST .../embeddings` with `model`, `input` array and `encoding_format=float`. CPU/GPU runtime may be on another host. No generative answer endpoint or Codex agent is used.

See `config.toml.example` for `[semantic]` and arbitrary `[semantic.headers]`. HTTP header values are treated as sensitive in diagnostics; config Debug does not expose them, redirects are disabled, and upstream response bodies are not copied into errors. No key is required unless configured through headers.

Chunks use UTF-8-safe bounded byte windows, preferring newline boundaries, with overlap. `chunk_bytes` is not a token budget: choose it conservatively for the provider/model and include prefixes/special tokens in the input allowance. The server never silently truncates rejected model inputs. It validates returned indices, dimensions, finite values and nonzero norms and normalizes vectors locally.

The first implementation uses exact cosine scans in the SQLite sidecar, appropriate for the present corpus, without an external vector DB. Larger corpora may require a different vector backend. Similarity is a relevance ranking, not a guarantee that a quote answers the question. Quotes always come from the current validated source revision, never from generated model prose.

## Interfaces

```sh
tb search --kind semantic 'restore after failure'
tb search --kind lexical 'exact_identifier'
tb search 'restore after failure' # hybrid
tb reindex --kind semantic /
tb reindex --kind lexical /manual
tb reindex / # hybrid
```

REST:

- `GET /api/search?q=...&kind=hybrid&dir=/manual`
- `POST /api/search`: JSON/form/multipart fields `q`, optional `dir`, `kind`, `with_status`.
- `POST /api/reindex`: JSON `{ "kind":"semantic", "path":"/" }`; same admin requirement as RPC reindex.

REST search returns the existing hit array unless `with_status=true`. It always sets `X-Tabularium-Lexical-Status` and `X-Tabularium-Semantic-Status` response headers, including for empty results.

JSON-RPC:

```json
{"jsonrpc":"2.0","id":1,"method":"search","params":{"query":"restore after failure","kind":"hybrid","with_status":true}}
{"jsonrpc":"2.0","id":2,"method":"reindex","params":{"kind":"semantic","path":"/"}}
```

RPC search preserves the legacy hit array by default. With `with_status=true`, it returns `{hits, lexical_status, semantic_status}`. MCP search uses this rich envelope by default. Existing hit fields remain; `line_end` and `revision` describe validated semantic excerpts. Ordinary snippets without source revision keep their legacy meaning.

Statuses include `ready`, `disabled`, `not_requested`, `unindexed`, `incompatible`, `partial`, `unavailable`. Partial means refreshes remain; hybrid results can still include current semantic quotes and lexical hits. CLI prints a concise diagnostic for incomplete/unavailable engines and retains the normal result list.

ACL, directory/file scope and source revision checks apply before semantic candidates consume the result limit. A denied document is not returned through either engine. The inference service receives source fragments during indexing and queries during search, so it is part of the trusted deployment boundary.

## Web UI

The search bar has independent scope (Local/Global) and kind
(Hybrid/Semantic/Lexical, default Hybrid) selectors. Changing kind during a
search refetches the committed query without changing its directory scope.

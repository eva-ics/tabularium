# REST API

Base path: `/api`. Document tree: **`/api/doc`**. `GET /api/doc` lists **root-level** entries (files and directories); deeper paths use additional URL segments (names or numeric ids).

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

- `GET /api/search?q=...`
- `POST /api/search` — JSON `{ "q": "..." }`, `application/x-www-form-urlencoded` (`q=...`), or `multipart/form-data` with field `q`.

Response: JSON array of `{ document_id, path, snippet, score, line_number? }`.

Errors: JSON `{ "error": "message" }` with `400` / `404` / `409` / `500` as appropriate.

# JSON-RPC methods (`POST /rpc`)

Transport: **JSON-RPC 2.0** over HTTP, `Content-Type: application/json`.  
Params are always a **JSON object** (empty object `{}` when none).  
Regex for `grep`: Rust [`regex`](https://docs.rs/regex) syntax on **UTF-8** document bodies (not PCRE).

Error `code` (baseline and Tabularium):

| Code | Meaning |
|------|---------|
| `-32700` | Parse error (invalid JSON body) |
| `-32600` | Invalid JSON-RPC version |
| `-32601` | Unknown method |
| `-32602` | Invalid params / validation (`InvalidInput`, `NotEmpty` detail in `message`) |
| `-32002` | Duplicate resource (`Duplicate` — e.g. document name already in parent directory); stable for clients |
| `-32603` | Other server error (`NotFound`, SQL, search, …; see `message`) |

Path rules:

- **`path`** is absolute (`/…`) for directory and file operations, or a **single segment** where the server accepts a root-level name (see `create_directory`).
- **No `\`** anywhere in a path string (future Windows-compat).
- Segment names: no `/` inside a segment, not pure decimal when used as a **name** (ids are decimal strings).

| Method | Params | Result |
|--------|--------|--------|
| `list_directory` | `{ path? }` — omit, empty, or `/` for root | `[{ id, kind, name, description?, created_at, modified_at, accessed_at, size_bytes?, recursive_file_count }]` |
| `create_directory` | `{ path, description? }` | `id` (number) |
| `delete_directory` | `{ path, recursive? }` | `null` (`recursive` true = delete files under tree then directory) |
| `rename_directory` | `{ path, new_path }` | `null` |
| `move_directory` | `{ path, new_parent, new_name }` | `null` |
| `list_documents` | `{ path }` | `[{ id, name, created_at_rfc3339, modified_at_rfc3339, accessed_at_rfc3339, size_bytes }]` |
| `create_document` | `{ path, content }` | `id` (number) |
| `put_document` | `{ path, content }` | `null` |
| `delete_document` | `{ path }` | `null` |
| `update_document` / `replace_document` | `{ path, content }` | `null` |
| `append_document` | `{ path, content }` | `null` (creates file and parent dirs if missing) |
| `say_document` | `{ path, from_id, content }` | `null` — **file must already exist** (`-32602` / `InvalidInput` with `say_document: document does not exist…`; use `append_document` or `put_document` to create) |
| `touch_document` | `{ path, modified_at? }` — without `modified_at`: create empty file (with parent dirs) if missing, else bump `modified_at` only (content and `created_at` unchanged); with `modified_at` (nanoseconds since Unix epoch, same wire type as elsewhere): set exact `modified_at` on file or directory, creating an empty file first if missing |
| `rename_document` | `{ path, new_name }` | `null` |
| `move_document` | `{ path, new_path }` | `null` (`new_path` = destination **file** path) |
| `get_document` / `cat` | `{ path }` | `{ id, path, content, created_at, modified_at, accessed_at, size_bytes }` |
| `get_document_ref` | `{ path }` | metadata only (no `content`) |
| `exists` | `{ path }` | `bool` |
| `search` | `{ query, path? }` | `[{ document_id, path, snippet, score, line_number? }]` (`path` = subtree filter, optional) |
| `reindex` | `{ path? }` or omit | `null` (full rebuild if `path` omitted/null/empty) |
| `head` | `{ path, lines? }` — omit `lines` for default 10; `lines: 0` returns no lines | `{ text }` |
| `tail` | `{ path, lines? }` — omit for default 10 last lines; `lines: 0` returns no lines; string `"+N"` = from line *N* | `{ text }` |
| `slice` | `{ path, start_line, end_line }` | `{ text }` (1-based inclusive lines) |
| `grep` | `{ path, pattern, max_matches? }` | `[{ line, text }]` (`max_matches` `0` = unlimited) |
| `wc` | `{ path }` | `{ bytes, lines, words, chars }` |
| `stat` | `{ path }` | `{ id, path, size_bytes, line_count, …timestamps }` |
| `test` | `{}` (no keys) | `{ product_name, product_version, uptime }` — diagnostics; `product_name` is `"tabularium"`, `product_version` is the **server** crate compile-time version, `uptime` is process uptime in nanoseconds (`u64`, saturates at max) |
| `wait` | `{ path }` | `null` when document body changes after the call begins; `-32602` with `"wait timed out"` at server long-poll ceiling |

Storage still uses an internal `categories` table / `category_id` for directory rows; that is **not** exposed on the wire.

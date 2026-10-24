# JSON document metadata

Each document has an independent JSON object, default `{}`. Nested objects, arrays and scalar values are allowed inside the object. Maximum serialized UTF-8 size is 65536 bytes. Metadata is not a document body, description or MIME type. Metadata updates replace the entire object and bump the shared document revision and modified time, invalidate the body cache, notify waiters, and refresh search. Use `only_if_revision` to prevent lost updates. Body edits retain metadata; copy preserves it; move/rename retain it naturally.

## Upgrade

At server startup, the SQLite migration takes `BEGIN IMMEDIATE`, checks `pragma_table_info('entries')`, and adds `metadata_json TEXT NOT NULL DEFAULT '{}'` only when absent. A JSON-object CHECK constraint protects stored data. DDL errors propagate and abort startup; they are never ignored. Existing bodies, IDs, revisions, times and ACLs are unchanged. Repeated starts are safe. Back up the source DB before deployment, using SQLite backup or stopping the server before copying all source state. No Tantivy schema migration is needed.

## API

`get_document_metadata {path}` returns `{metadata, revision}`. `set_document_metadata {path, metadata, only_if_revision?}` replaces the object. Read/write ACLs are the same as the document. Available through JSON-RPC and MCP. REST: `GET /api/metadata/<path>` and `PUT /api/metadata/<path>` with `{metadata, only_if_revision?}`. REST and RPC document-body reads also include `metadata`.

## CLI

```
tb meta /notes/example.md
tb meta /notes/example.md --export example.metadata.json
tb meta /notes/example.md --import example.metadata.json
tb meta /notes/example.md --edit
```

Editing uses `$EDITOR` on a temporary JSON file and saves with the fetched revision. Invalid JSON and conflicting edits do not overwrite server metadata; the edited temporary file is retained with its path in the error. Explicit export refuses to overwrite an existing local file.

Normal recursive and single-file import recognizes `<filename>.metadata.json` beside a document, including its original basename when `--name` renames the imported document. A paired sidecar is not imported as a standalone document. An unpaired file with that suffix remains an ordinary document. Invalid sidecars are validated before creating the paired body. Export writes a pretty-printed sidecar for nonempty metadata. Empty metadata needs no sidecar. A document named like another document's paired sidecar is a collision: export reports an error instead of creating an ambiguous tree. Imports are create-only, not an overwrite transaction; remote or disk failures are reported and may leave a partially imported/exported tree. Keep original exports until verification succeeds.

## Search and UI

Metadata keys and values join the lexical auxiliary `description` field, without changing stored descriptions or the Tantivy schema. Existing filename/description semantics apply: metadata-only hits show an exact excerpt from the body, not a fabricated body quote containing metadata. Metadata does not enter semantic embeddings; those continue to use only source content. Changes requeue semantic indexing because document revisions are shared. New installations and upgrades start with empty metadata, so existing indices need no rebuild solely for this migration. Normal reindex includes metadata.

Web preview has a wrapping full-path row and a separate wrapping action row. Meta shows read-only, escaped JSON; editing remains CLI/API only.

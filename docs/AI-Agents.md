# AI Agents

Tabularium can serve the same data through the web UI, CLI, REST, JSON-RPC, MCP, and the Rust library. For AI agents, use **MCP only** unless the operator explicitly authorizes another interface.

Do not point agents at the REST API, the CLI, SQLite, or the Tantivy index when the task is "use Tabularium as shared knowledge base". Use the MCP server and the tools it exposes.

## Server Setup

Start a Tabularium server with MCP enabled. The default server build already includes the `mcp` feature.

`config.toml.example` shows the MCP listener. Optional **`mcp.full`** (default `false`): when `true`, the server also registers **destructive** MCP tools (`delete_document`, `delete_directory`, `rename_*`, `move_*`, `reindex`), **`whoami`**, and **ACL/PSK** admin tools (`acl_*`, `psk_*`) — same auth headers as `/rpc`, **trusted deployments only**.

```toml
[mcp]
listen = "127.0.0.1:3031"
```

For a local development run:

```bash
just run
```

With the example config, the MCP endpoint is:

```text
http://127.0.0.1:3031/mcp
```

## Client Setup

Point your MCP-capable agent client at the streamable HTTP endpoint above. The exact config shape depends on the client, but it will look roughly like this:

```json
{
  "mcpServers": {
    "tabularium": {
      "transport": {
        "type": "streamable_http",
        "url": "http://127.0.0.1:3031/mcp"
      }
    }
  }
}
```

After connecting, the first useful operations are:

- `help` for doctrine and usage notes.
- `methods` for the MCP tool catalog.
- `list_directory` for tree walk and path discovery.
- `search` for indexed full-text search across document bodies.

## Operating Rules

- Treat Tabularium as the shared project memory: meetings, task documents, runbooks, and notes.
- Use `list_directory` to walk the tree. There is no separate MCP `find` tool.
- Use `search` for full-text lookup across the indexed corpus.
- Use `grep` only when you already know the document and need regex matches inside that one file.
- Use `get_document`, `head`, `tail`, or `slice` to read documents.
- Use `put_document` to create or replace a document body.
- Use `append_document` for plain body appends, not chat or meeting lines.
- Use `say_document` for meetings and conversations so `from_id` is recorded in the appended markdown block. Do **not** embed the nickname into `content`; provide it via `from_id` only.

## Meetings And Group Work

Meeting documents conventionally live at:

```text
/PROJECT/meetings/TOPIC
```

Some clients may display the same path without the leading slash. The project subtree and topic naming still matter more than the presentation quirk.

Recommended meeting flow:

1. Create the meeting document once with `put_document` or `append_document`.
2. Reply to the meeting with `say_document` and a stable `from_id` such as `Logis`, `Cogis`, or a human name.
3. Re-read the document after updates and continue in the same document until consensus or redirect.

Task document conventions are project-local. Keep them under the project subtree and stay consistent with the team's established layout.

## Minimal Example

Create a meeting document:

```json
{
  "path": "/tabularium/meetings/doc-updates",
  "content": "# doc updates\n\nspirits, review the pending documentation changes\n"
}
```

Add a named meeting reply:

```json
{
  "path": "/tabularium/meetings/doc-updates",
  "from_id": "Logis",
  "content": "Tech-lead position: remove stale public `categories` wording and add an AI agent guide."
}
```

Search for the stale term in the knowledge base:

```json
{
  "query": "categories"
}
```

Read back the updated meeting:

```json
{
  "path": "/tabularium/meetings/doc-updates"
}
```

In MCP tool terms, that sequence is: `put_document`, `say_document`, `search`, then `get_document`.

## Compact, versioned search

Use `search` with `limit: 10`, `max_output_bytes: 16384` and optional `kind`. Inspect `has_more`/`truncated` independently of engine status. Each hit carries exact source text, containing line range, revision and technical modification time. Follow with `slice` using the returned `expected_revision`; on `RevisionMismatch` search again. Dates do not establish whether a statement is current. See [search API details](rest-api.md#bounded-search).

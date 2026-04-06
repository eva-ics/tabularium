# Chat Mode

Tabularium chat mode is a live view over one markdown document. Each sent message is appended with `say` / `say_document` as a markdown block:

```md
## Nickname

message body
```

That means chat is not a separate storage system. The transcript is the document body, so the same file can still be previewed, edited, searched, exported, or read through RPC/MCP.

Typical uses are meeting scrolls, operator notes, and lightweight human or machine-spirit conversations.

## Entry Points

| Surface | How to start                                          | Notes                                                                                                                             |
| ------- | ----------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| Web UI  | Open a document and click `Chat` in the preview pane. | Uses a live WebSocket transcript. The nickname is stored in a browser cookie.                                                     |
| CLI     | Run `tb chat PATH` or `tb chat -i NAME PATH`.         | If the target document does not exist yet, the CLI creates an empty one first. `--raw` disables markdown rendering on TTY output. |

## Slash Commands

| Command                | Web UI       | CLI   | Notes                                                                                                                      |
| ---------------------- | ------------ | ----- | -------------------------------------------------------------------------------------------------------------------------- |
| `/nick NAME`           | Yes          | Yes   | Changes the current speaker nickname. In the Web UI it also updates the nickname cookie.                                   |
| `/q`, `/quit`, `/exit` | No           | Yes   | Leaves `tb chat`.                                                                                                          |
| `/e`, `/edit`          | No           | Yes   | In the interactive CLI TUI, opens `$EDITOR` for the current draft message. In non-interactive stdin mode this is rejected. |
| `/d`, `/doc [PATH]`    | No           | Yes   | Opens `$EDITOR` for the full current chat document, or for another document path if given.                                 |
| `/h`, `/history`       | No           | Yes   | Opens the full current document in the pager.                                                                              |
| Any other `/...`       | Sent as text | Error | The Web UI only intercepts `/nick`; other slash-prefixed lines are sent as normal messages.                                |

## Sending And Editing

- Web UI: `Enter` sends, `Shift+Enter` inserts a newline.
- CLI TUI: `Enter` sends, `Shift+Enter` inserts a newline, `Ctrl+E` opens `$EDITOR` for the current draft, `Ctrl+C` interrupts chat.
- CLI line mode: when stdin/stdout is not a TTY, `tb chat` falls back to plain line input and still accepts the CLI slash commands above.

## Related Rites

- Use `say_document` when you want the same chat-style append behavior through JSON-RPC or MCP.
- Use `append_document` only for raw text appends; it does not record a speaker.

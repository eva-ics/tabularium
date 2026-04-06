# GFM table fixture

Shared across CLI `cat`, CLI chat TUI, Web preview, and Web chat (`meetings/tables`).

| Domain | Use | Avoid |
|--------|-----|-------|
| Async | `tokio` (full), `async-trait` | `async-std`, manual futures |
| Errors | `thiserror` + crate `Error` enum | `anyhow` for library code |

Wide row for column-width stress:

| Short | This is a much longer cell value for layout testing |
|-------|-----------------------------------------------------|
| x | y |

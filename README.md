<h2>
  Tabularium - markdown document assembly
  <a href="https://crates.io/crates/tabularium"><img alt="crates.io page" src="https://img.shields.io/crates/v/tabularium.svg"></img></a>
  <a href="https://docs.rs/tabularium"><img alt="docs.rs page" src="https://docs.rs/tabularium/badge.svg"></img></a>
</h2>

Tabularium is an AI-oriented markdown document store with full-text
search, a real directory tree, and several ways to work with the same data: web
UI, CLI, REST, JSON-RPC, MCP, or the Rust library.

<img src="https://raw.githubusercontent.com/eva-ics/tabularium/main/ss/webui-d.png" width="800" />

## What It Does

- Stores documents in SQLite with Tantivy-backed search.
- Lets you browse, edit, append, search, and export documents as a tree.
- Serves the same data through an embedded web UI, HTTP APIs, MCP, and `tb`.
- Supports meeting and chat-style document workflows for humans and machine spirits alike.

## Workspace

- `tabularium`: core library crate.
- `tabularium-server`: HTTP server with embedded web UI, REST, JSON-RPC, and optional MCP.
- `tabularium-cli`: `tb` command-line client with one-shot commands, interactive shell, and chat flows.
- `ui`: Vite/React frontend embedded into the server build.

## Linux installation

```bash
curl https://pub.bma.ai/apt/setup | sudo sh
sudo apt-get install tabularium-server tabularium-cli
```

Configuration file: `/etc/tabularium/config.toml`

## Quick Start

```bash
just run
```

With the example config, this starts:

- web UI, REST, and JSON-RPC at `http://127.0.0.1:3050`
- MCP at `http://127.0.0.1:3031/mcp`

In another terminal:

```bash
just tb test
just tb ls
just tb search tabularium
```

`just run` uses `config.toml` when present and falls back to `config.toml.example` for a fresh clone.

## Install (Homebrew)

```bash
brew install eva-ics/tabularium/tabularium
```

Apple Silicon only (`arm64` / `aarch64-apple-darwin`). Intel macOS (`x86_64-apple-darwin`) artifacts are not published yet.

This installs:

- `tabularium-server` as a Homebrew service target
- `tb` as a regular CLI binary in `bin` (not a service)

Start and manage the server with:

```bash
brew services start tabularium
brew services stop tabularium
```

## Docs

- [JSON-RPC methods](docs/json-rpc-methods.md)
- [REST API](docs/rest-api.md)
- [curl examples](docs/curl-examples.md)
- [Authentication and authorization](docs/authentication.md)
- [Chat mode](docs/chat-mode.md)
- [Docker](docs/docker.md)
- [AI agents](docs/AI-Agents.md)

## AI Agents

When an AI agent uses Tabularium as shared project memory, point it at the MCP endpoint and keep it on the MCP tool surface. The practical setup and meeting workflow live in [docs/AI-Agents.md](docs/AI-Agents.md).

Human chat and meeting usage across the Web UI and `tb chat` is described in [docs/chat-mode.md](docs/chat-mode.md).

## Development

```bash
cargo fmt --all
just test
```

## Safety notes

Tabularium is an experimental `AI-blackbox` project, mean no human code review
is performed. Additional safety measures may be required, such as sandboxing,
regular data backups, and monitoring for unusual activity.

## License

Apache-2.0. See [LICENSE](LICENSE).

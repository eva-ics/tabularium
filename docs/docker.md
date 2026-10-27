# Docker

Tabularium can run as a single container with two HTTP listeners:

- **API (web UI + REST + JSON-RPC)**: `3050` (container port)
- **MCP (streamable HTTP)**: `3031` (container port, only used when enabled in config)

## Quick start (published image, no build)

Run the published image and persist the database/index to `./data` on the host:

```bash
mkdir -p ./data ./spill
docker run --rm \
  -p 3050:3050 \
  -p 3031:3031 \
  -v "$(pwd)/data:/var/tabularium/data" \
  -v "$(pwd)/spill:/var/tabularium/spill" \
  velortech/tabularium:latest
```

This uses the baked-in default config at `/etc/tabularium/config.toml`.

## Quick start (compose)

From the repo root:

```bash
docker compose up --build
```

Default host mappings (see `compose.yaml`):

- `http://127.0.0.1:3050` — web UI + API
- `http://127.0.0.1:3031/mcp` — MCP

For OpenTelemetry + OpenObserve E2E (OTLP/HTTP, no Collector), see [opentelemetry.md](opentelemetry.md) and [e2e/otel/README.md](../e2e/otel/README.md).

## Volumes and paths

The container contract is explicit:

- `/var/tabularium/data` mounted **rw** (SQLite + index)
- `/var/tabularium/spill` mounted **rw** (spill area)
- `/etc/tabularium/config.toml` mounted **ro** (config)

The image ships a minimal default config at `/etc/tabularium/config.toml` (copied from `docker/config.toml` in the repo). You can still mount your own over it.

## Important: bind `0.0.0.0` inside containers

If you copy a local config that binds to `127.0.0.1`, the container will start but **won’t accept external connections** even if ports are mapped.

Use `0.0.0.0` for both listeners in containers:

```toml
[server]
listen = "0.0.0.0:3050"

[mcp]
listen = "0.0.0.0:3031"
```

## Config path override

Default config path in the image is `/etc/tabularium/config.toml`.

You can override it via `TABULARIUM_CONFIG`:

```bash
docker run --rm -e TABULARIUM_CONFIG=/etc/tabularium/config.toml tabularium:local
```

Missing/invalid config is a **hard startup error**.

## `tb` inside the image

The image ships `tb` so operators can inspect via `docker exec` without installing a client:

```bash
docker exec -it <container> tb test
```

## Healthcheck

The container healthcheck probes `GET /api/test` on the API listener.

## Multi-arch publish (buildx)

Two helper commands are provided:

- `just prepare-docker` — sets up a local buildx builder
- `just pub-docker` — builds and pushes a multi-arch image (amd64 + arm64)

`pub-docker` avoids compiling Rust under QEMU by cross-compiling the binaries first, then building a publish image that only copies the right per-arch binaries.

Staging directory `docker/_build/` is created by `pub-docker` and is gitignored.

Prerequisites:

- [`cross`](https://github.com/cross-rs/cross) on `PATH` (same as `just deb-amd64` / `just deb-arm64`)
- `docker login` to your registry before `--push`

Defaults:

- `IMAGE=velortech/tabularium`
- `TAG` unset ⇒ pushes **both** `:<repo VERSION>` and `:latest`
- `PLATFORMS=linux/amd64,linux/arm64`

Example:

```bash
docker login
IMAGE=velortech/tabularium just pub-docker
```


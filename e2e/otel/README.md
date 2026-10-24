# OpenTelemetry E2E (Tabularium → OpenObserve via OTLP/HTTP)

Local stack with **no Collector**:

```text
Tabularium  --OTLP/HTTP-->  OpenObserve
```

## Prerequisites

- Docker + Docker Compose v2
- Ports free: `3050`, `3031`, `5080`

## Start

From the repository root:

```bash
# Basic auth header for OTLP (root@example.com:Complexpass#123)
export OO_BASIC_AUTH="$(printf '%s' 'root@example.com:Complexpass#123' | base64)"

docker compose -f e2e/otel/compose.yaml --env-file e2e/otel/.env.example up --build
```

Or run the verifier as a one-shot after services are healthy:

```bash
docker compose -f e2e/otel/compose.yaml --env-file e2e/otel/.env.example up --build -d
docker compose -f e2e/otel/compose.yaml run --rm e2e
```

## Endpoints

| Service | URL |
|---------|-----|
| Tabularium UI/API | http://127.0.0.1:3050 |
| Tabularium MCP | http://127.0.0.1:3031/mcp |
| OpenObserve UI | http://127.0.0.1:5080 |

OpenObserve local credentials (e2e only): `root@example.com` / `Complexpass#123`.

## Inspect traces

1. Open http://127.0.0.1:5080 and sign in.
2. Go to **Traces** (org `default`).
3. Filter `service_name = tabularium`.
4. Expected hierarchy (names may include crate targets):

```text
mcp.tools/call
└── search / Database::search / search_hits
    ├── files_display_batch   (SQLite)
    └── SearchIndex::search_scored
        ├── search_parse
        └── search_tantivy
```

## Load test

`scripts/run_e2e.py` runs search workloads at concurrency **10 / 50 / 100** and prints JSON lines with `success`, `errors`, `p50`, `p95`, `p99`. No SLO thresholds are asserted — values are measured only.

After load, re-open OpenObserve Traces to see which child span grew with concurrency.

## Cleanup

```bash
docker compose -f e2e/otel/compose.yaml down -v
```

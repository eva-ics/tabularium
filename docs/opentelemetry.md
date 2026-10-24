# OpenTelemetry (OTLP)

Tabularium exports traces and metrics with the OpenTelemetry SDK over **OTLP/HTTP protobuf**, layered onto the existing `tracing` / `#[instrument]` spans. There is **no** OpenTelemetry Collector in the default path:

```text
tabularium-server  --OTLP/HTTP-->  OpenObserve (or any OTLP/HTTP backend)
```

## Enable / disable

Export is **off by default**. The process starts with the usual fmt subscriber when OTLP is not configured.

Enable by setting a non-empty OTLP endpoint (and not disabling the SDK):

```bash
export OTEL_SERVICE_NAME=tabularium
export OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf
export OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=http://openobserve:5080/api/default/v1/traces
export OTEL_EXPORTER_OTLP_METRICS_ENDPOINT=http://openobserve:5080/api/default/v1/metrics
export OTEL_EXPORTER_OTLP_HEADERS='Authorization=Basic …'
```

Disable explicitly:

```bash
export OTEL_SDK_DISABLED=true
```

Or omit the endpoint variables.

Supported (standard) variables include:

| Variable | Role |
|----------|------|
| `OTEL_SERVICE_NAME` | Service name in traces/metrics (default `tabularium`) |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Base OTLP endpoint |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | Traces URL (takes precedence for traces) |
| `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` | Metrics URL |
| `OTEL_EXPORTER_OTLP_HEADERS` | Extra headers (`k=v,k2=v2`) |
| `OTEL_EXPORTER_OTLP_PROTOCOL` / `OTEL_EXPORTER_OTLP_TRACES_PROTOCOL` | Expect `http/protobuf` |
| `OTEL_SDK_DISABLED` | `true` forces fmt-only mode |

Do not put OpenObserve URLs, tenants, or credentials in source code — only in deployment env / compose.

## OpenObserve example

```bash
OTEL_SERVICE_NAME=tabularium
OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=http://openobserve:5080/api/default/v1/traces
OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf
OTEL_EXPORTER_OTLP_HEADERS=Authorization=Basic <base64(email:password)>
```

Local Docker E2E: see [e2e/otel/README.md](../e2e/otel/README.md).

## Trace hierarchy

Existing `#[instrument]` spans in `tabularium` (Database, SqliteStorage, SearchIndex) are reused. MCP adds a root span:

```text
mcp.tools/call                    (mcp.tool / mcp.method / otel.status_code)
└── Database::search / search_hits
    ├── files_display_batch       (SQLite)
    └── SearchIndex::search_scored
        ├── search_parse
        └── search_tantivy
```

### MCP attributes

- `mcp.method` / `rpc.method` / `mcp.tool` — tool / JSON-RPC method name
- `otel.status_code` — `OK` or `ERROR`
- `error.type` — `auth_error` or `tool_error` when failed

Request bodies, search queries, and document contents are **not** recorded as span attributes or span events (RPC param previews stay on the fmt log only).

Application-level tool failures (`Err(String)` from MCP tools) mark the span as error even when the HTTP transport returns 200.

## Metrics

When OTLP is enabled:

- `mcp.client.operation.duration` (histogram, seconds)
- `mcp.tool.calls` / `mcp.tool.errors` / `mcp.tool.active`
- `mcp.wait.timeouts` — long-poll `wait` timeouts (if that counter is registered)
- Tokio gauges: `tokio.runtime.workers`, `tokio.runtime.alive_tasks`, `tokio.runtime.global_queue_depth`

Percentiles (p50/p95/p99) are computed in the backend (OpenObserve), not in the Rust process.

## Shutdown

On process exit the server flushes/shuts down the tracer and meter providers with a short wall-clock timeout so shutdown cannot hang indefinitely.

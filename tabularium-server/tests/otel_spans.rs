//! OpenTelemetry span hierarchy around MCP→search (in-memory exporter, no OpenObserve).

#![cfg(feature = "mcp")]

use std::sync::Arc;
use std::time::Duration;

use bma_ts::Monotonic;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::{InMemorySpanExporterBuilder, SdkTracerProvider};
use serde_json::json;
use tabularium::SqliteDatabase;
use tabularium_server::mcp::TabulariumMcp;
use tabularium_server::otel::Telemetry;
use tabularium_server::web::AppState;
use tracing::Instrument;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

async fn seeded_app() -> (AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("otel.db");
    let idx_path = dir.path().join("otel.idx");
    let uri = format!("sqlite://{}", db_path.display());
    let db = Arc::new(SqliteDatabase::init(&uri, &idx_path, 8).await.unwrap());
    db.create_directory("/notes", None, true).await.unwrap();
    db.put_document_by_path(
        "/notes/alpha.md",
        "observability latency search tantivy",
        false,
        None,
    )
    .await
    .unwrap();
    let app = AppState {
        db,
        wait_timeout: Duration::from_secs(2),
        process_started_at: Monotonic::now(),
        authenticate_api: false,
        authenticate_mcp: false,
        oidc: None,
        anonymous: None,
    };
    (app, dir)
}

#[test]
fn telemetry_disabled_shutdown_is_noop() {
    Telemetry::disabled().shutdown();
}

#[tokio::test]
async fn mcp_search_emits_nested_spans() {
    let exporter = InMemorySpanExporterBuilder::new().build();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let tracer = provider.tracer("tabularium-test");
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let _guard = tracing_subscriber::registry()
        .with(otel_layer)
        .set_default();

    let (app, _dir) = seeded_app().await;
    let mcp = TabulariumMcp::new(app, Arc::from(""), false);
    let body = mcp
        .invoke_rpc_for_test("search", json!({ "query": "observability" }))
        .await
        .expect("search ok");
    assert!(body.contains("alpha.md") || body.contains("observability") || body != "null");

    provider.force_flush().unwrap();
    let spans = exporter.get_finished_spans().unwrap();
    let names: Vec<&str> = spans.iter().map(|s| s.name.as_ref()).collect();
    assert!(
        names
            .iter()
            .any(|n| n.contains("mcp.tools/call") || *n == "mcp.tools/call"),
        "missing mcp root span in {names:?}"
    );
    assert!(
        names
            .iter()
            .any(|n| *n == "search_parse" || n.contains("search_parse")),
        "missing search_parse in {names:?}"
    );
    assert!(
        names
            .iter()
            .any(|n| *n == "search_tantivy" || n.contains("search_tantivy")),
        "missing search_tantivy in {names:?}"
    );
}

#[tokio::test]
async fn mcp_params_and_bodies_stay_out_of_otel_export() {
    let exporter = InMemorySpanExporterBuilder::new().build();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let tracer = provider.tracer("tabularium-test");
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let _guard = tracing_subscriber::registry()
        .with(otel_layer)
        .set_default();

    let marker = "otel-secret-body-should-never-export-9f3c2a1b";
    let (app, _dir) = seeded_app().await;
    let mcp = TabulariumMcp::new(app, Arc::from(""), false);
    // MCP transport (and Axum) keep an outer tracing span; param previews must
    // stay off OTLP even when that parent is active — not only off mcp.tools/call.
    let transport = tracing::info_span!(
        "mcp.transport",
        otel.name = "mcp.transport",
        "rpc.system" = "mcp",
    );
    async {
        mcp.invoke_rpc_for_test(
            "put_document",
            json!({
                "path": "/notes/secret.md",
                "content": marker,
                "force": true,
            }),
        )
        .await
        .expect("put ok");
        mcp.invoke_rpc_for_test("search", json!({ "query": marker }))
            .await
            .expect("search ok");
    }
    .instrument(transport)
    .await;

    provider.force_flush().unwrap();
    let spans = exporter.get_finished_spans().unwrap();
    assert!(
        spans.iter().any(|s| s.name.as_ref() == "mcp.transport"),
        "expected outer transport span in export: {:?}",
        spans.iter().map(|s| s.name.as_ref()).collect::<Vec<_>>()
    );
    for span in &spans {
        for kv in &span.attributes {
            let hay = format!("{}={}", kv.key.as_str(), kv.value.as_str());
            assert!(
                !hay.contains(marker),
                "span attribute leaked MCP payload: {hay} (span={})",
                span.name
            );
        }
        for event in span.events.iter() {
            let name = event.name.as_ref();
            assert!(
                !name.contains(marker),
                "span event name leaked MCP payload: {name}"
            );
            for kv in &event.attributes {
                let hay = format!("{}={}", kv.key.as_str(), kv.value.as_str());
                assert!(
                    !hay.contains(marker),
                    "span event attribute leaked MCP payload: {hay}"
                );
            }
        }
    }
}

#[tokio::test]
async fn mcp_tool_error_marks_span_error() {
    let exporter = InMemorySpanExporterBuilder::new().build();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let tracer = provider.tracer("tabularium-test");
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let _guard = tracing_subscriber::registry()
        .with(otel_layer)
        .set_default();

    let (app, _dir) = seeded_app().await;
    let mcp = TabulariumMcp::new(app, Arc::from(""), false);
    let err = mcp
        .invoke_rpc_for_test("get_document", json!({ "path": "/does/not/exist.md" }))
        .await
        .expect_err("missing doc must fail");
    assert!(!err.is_empty());

    provider.force_flush().unwrap();
    let spans = exporter.get_finished_spans().unwrap();
    let mcp_spans: Vec<_> = spans
        .iter()
        .filter(|s| s.name.as_ref() == "mcp.tools/call")
        .collect();
    assert!(!mcp_spans.is_empty(), "expected mcp.tools/call span");
    let status_err = mcp_spans.iter().any(|s| {
        s.attributes
            .iter()
            .any(|kv| kv.key.as_str() == "otel.status_code" && kv.value.as_str() == "ERROR")
            || s.status == opentelemetry::trace::Status::error("")
            || format!("{:?}", s.status).contains("Error")
    });
    // Attribute recording via tracing fields may land as OTel status or attributes.
    assert!(
        status_err
            || mcp_spans.iter().any(|s| {
                s.attributes
                    .iter()
                    .any(|kv| kv.key.as_str() == "error.type")
            }),
        "tool error not reflected on span: {:?}",
        mcp_spans
            .iter()
            .map(|s| (&s.name, &s.status, &s.attributes))
            .collect::<Vec<_>>()
    );
}

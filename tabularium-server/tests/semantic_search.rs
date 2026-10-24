//! HTTP contracts: engine selection, exact excerpts, compatibility and fallback.
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use bma_ts::Monotonic;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tabularium::{SemanticConfig, SqliteDatabase};
use tabularium_server::web::{AppState, router};

async fn embed(
    State(offline): State<Arc<AtomicBool>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    assert_eq!(headers["x-test-header"], "test-only");
    if offline.load(Ordering::SeqCst) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let data: Vec<_> = body["input"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, _)| json!({"index":i,"embedding":[1.0,0.0]}))
        .collect();
    Json(json!({"data":data})).into_response()
}
#[tokio::test]
async fn semantic_transports_reindex_status_and_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let offline = Arc::new(AtomicBool::new(false));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/v1/embeddings", post(embed))
        .with_state(offline.clone());
    let model_task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let cfg: SemanticConfig = serde_json::from_value(
        json!({"base_url":url,"model":"test","headers":{"x-test-header":"test-only"}}),
    )
    .unwrap();
    let db = Arc::new(
        SqliteDatabase::init(
            format!("sqlite://{}", dir.path().join("source.db").display()),
            dir.path().join("lexical"),
            0,
        )
        .await
        .unwrap()
        .with_semantic(cfg, &dir.path().join("semantic"))
        .await
        .unwrap(),
    );
    db.create_file_in_directory("/", "recovery", "Title\nTake the backup.\n")
        .await
        .unwrap();
    let app = router(AppState {
        db,
        wait_timeout: Duration::from_secs(1),
        process_started_at: Monotonic::now(),
        authenticate_api: false,
        authenticate_mcp: false,
        oidc: None,
        anonymous: None,
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server_task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::new();
    let r = client
        .post(format!("{base}/api/reindex"))
        .json(&json!({"kind":"hybrid"}))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    let bodies = [
        client
            .get(format!(
                "{base}/api/search?q=recover&kind=semantic&with_status=true"
            ))
            .send()
            .await
            .unwrap(),
        client
            .post(format!("{base}/api/search"))
            .json(&json!({"q":"recover","kind":"semantic","with_status":true}))
            .send()
            .await
            .unwrap(),
        client
            .post(format!("{base}/api/search"))
            .header("content-type", "application/x-www-form-urlencoded")
            .body("q=recover&kind=semantic&with_status=true")
            .send()
            .await
            .unwrap(),
        client
            .post(format!("{base}/api/search"))
            .multipart(
                reqwest::multipart::Form::new()
                    .text("q", "recover")
                    .text("kind", "semantic")
                    .text("with_status", "true"),
            )
            .send()
            .await
            .unwrap(),
    ];
    for r in bodies {
        assert!(r.status().is_success());
        assert_eq!(r.headers()["x-tabularium-lexical-status"], "not_requested");
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["hits"][0]["snippet"], "Title\nTake the backup.\n");
        assert_eq!(v["hits"][0]["line_number"], 1);
        assert_eq!(v["hits"][0]["line_end"], 2);
        assert!(v["hits"][0]["revision"].is_string());
    }
    let v: Value = client.post(format!("{base}/rpc")).json(&json!({"jsonrpc":"2.0","id":1,"method":"search","params":{"query":"recover","kind":"semantic","with_status":true}})).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["result"]["hits"][0]["path"], "/recovery");
    let legacy: Value = client
        .get(format!("{base}/api/search?q=backup&kind=lexical"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(legacy.is_array());
    let r = client
        .get(format!("{base}/api/search?q=backup&kind=bogus"))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_client_error());
    offline.store(true, Ordering::SeqCst);
    let r = client
        .get(format!("{base}/api/search?q=backup&with_status=true"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.headers()["x-tabularium-semantic-status"], "unavailable");
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["hits"][0]["path"], "/recovery");
    server_task.abort();
    model_task.abort();
}

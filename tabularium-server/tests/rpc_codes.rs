//! JSON-RPC 2.0 error code mapping (method not found / invalid params).

use std::sync::Arc;
use std::time::Duration;

use bma_ts::Monotonic;
use serde_json::{Value, json};
use tabularium::SqliteDatabase;
use tabularium::jsonrpc_codes::DUPLICATE_RESOURCE;
use tabularium_server::web::{AppState, router};
use tokio::net::TcpListener;

#[tokio::test]
async fn unknown_rpc_method_returns_minus_32601() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("t.db");
    let idx_path = dir.path().join("t.idx");
    let uri = format!("sqlite://{}", db_path.display());
    let db = Arc::new(SqliteDatabase::init(&uri, &idx_path, 8).await.unwrap());
    let app = router(AppState {
        db,
        wait_timeout: Duration::from_secs(3600),
        process_started_at: Monotonic::now(),
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let base = format!("http://{}", addr);
    let client = reqwest::Client::new();
    let body = json!({
        "jsonrpc": "2.0",
        "method": "definitely_not_a_real_method",
        "params": {},
        "id": 7_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], -32601);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown method")
    );
}

#[tokio::test]
async fn invalid_rpc_params_return_minus_32602() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("t2.db");
    let idx_path = dir.path().join("t2.idx");
    let uri = format!("sqlite://{}", db_path.display());
    let db = Arc::new(SqliteDatabase::init(&uri, &idx_path, 8).await.unwrap());
    let app = router(AppState {
        db,
        wait_timeout: Duration::from_secs(3600),
        process_started_at: Monotonic::now(),
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let base = format!("http://{}", addr);
    let client = reqwest::Client::new();
    // `create_directory` requires string `path`; wrong type → InvalidInput → -32602.
    let body = json!({
        "jsonrpc": "2.0",
        "method": "create_directory",
        "params": { "path": 42 },
        "id": 8_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], -32602);
}

#[tokio::test]
async fn rpc_duplicate_create_document_returns_duplicate_resource_code() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("t3.db");
    let idx_path = dir.path().join("t3.idx");
    let uri = format!("sqlite://{}", db_path.display());
    let db = Arc::new(SqliteDatabase::init(&uri, &idx_path, 8).await.unwrap());
    let app = router(AppState {
        db,
        wait_timeout: Duration::from_secs(3600),
        process_started_at: Monotonic::now(),
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let base = format!("http://{}", addr);
    let client = reqwest::Client::new();
    let cat = "rpc_dup_code_cat";
    let path = format!("/{cat}/same_doc");
    let body = json!({
        "jsonrpc": "2.0",
        "method": "create_directory",
        "params": { "path": format!("/{cat}"), "description": null },
        "id": 10_i64,
    });
    client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();

    let create = json!({
        "jsonrpc": "2.0",
        "method": "create_document",
        "params": { "path": path, "content": "a" },
        "id": 11_i64,
    });
    client
        .post(format!("{base}/rpc"))
        .json(&create)
        .send()
        .await
        .unwrap();

    let r = client
        .post(format!("{base}/rpc"))
        .json(&create)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], DUPLICATE_RESOURCE);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("duplicate")
    );
}

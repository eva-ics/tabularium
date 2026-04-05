//! `test` JSON-RPC diagnostics.

use std::sync::Arc;
use std::time::Duration;

use bma_ts::Monotonic;
use serde_json::{Value, json};
use tabularium::SqliteDatabase;
use tabularium_server::web::{AppState, router};
use tokio::net::TcpListener;

#[tokio::test]
async fn rpc_test_returns_identity_and_uptime() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("t_test.db");
    let idx_path = dir.path().join("t_test.idx");
    let uri = format!("sqlite://{}", db_path.display());
    let started = Monotonic::now();
    let db = Arc::new(SqliteDatabase::init(&uri, &idx_path, 8).await.unwrap());
    let app = router(AppState {
        db,
        wait_timeout: Duration::from_secs(3600),
        process_started_at: started,
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
        "method": "test",
        "params": {},
        "id": 1_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let v: Value = r.json().await.unwrap();
    assert!(v.get("error").is_none());
    let res = v.get("result").unwrap();
    assert_eq!(res["product_name"], "tabularium");
    assert_eq!(
        res["product_version"].as_str().unwrap(),
        env!("CARGO_PKG_VERSION")
    );
    let uptime = res["uptime"]
        .as_u64()
        .expect("uptime must be u64 JSON number");
    assert!(uptime > 0, "expected some elapsed nanoseconds");
}

#[tokio::test]
async fn rpc_test_rejects_extra_params() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("t_test2.db");
    let idx_path = dir.path().join("t_test2.idx");
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
        "method": "test",
        "params": { "nope": true },
        "id": 2_i64,
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

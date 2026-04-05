use std::sync::Arc;
use std::time::Duration;

use bma_ts::Monotonic;
use tabularium::SqliteDatabase;
use tabularium_server::web::{AppState, router};
use tokio::net::TcpListener;

#[tokio::test]
async fn rest_root_listing_and_rpc_list_directory() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("t.db");
    let idx_path = dir.path().join("t.idx");
    let uri = format!("sqlite://{}", db_path.display());
    let db = Arc::new(SqliteDatabase::init(&uri, &idx_path, 8).await.unwrap());
    let app = router(AppState {
        db: db.clone(),
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

    let r = client.get(format!("{base}/api/doc")).send().await.unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let v: serde_json::Value = r.json().await.unwrap();
    assert!(v.is_array());

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "list_directory",
        "params": {},
        "id": 1
    });
    let r2 = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r2.status(), reqwest::StatusCode::OK);
    let resp: serde_json::Value = r2.json().await.unwrap();
    assert!(resp.get("result").is_some());

    // multipart/form-data search (field `q`)
    let client_m = reqwest::Client::new();
    let form = reqwest::multipart::Form::new().text("q", "test");
    let r3 = client_m
        .post(format!("{base}/api/search"))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(r3.status(), reqwest::StatusCode::OK);

    let r4 = client
        .post(format!("{base}/api/search"))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body("q=hello")
        .send()
        .await
        .unwrap();
    assert_eq!(r4.status(), reqwest::StatusCode::OK);

    let r5 = client.get(format!("{base}/")).send().await.unwrap();
    assert_eq!(r5.status(), reqwest::StatusCode::OK);
    let ct = r5
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.starts_with("text/html"),
        "GET / must be HTML for SPA, got {ct:?}"
    );

    let r6 = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let ct_rpc = r6
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    assert!(
        ct_rpc.contains("json"),
        "POST /rpc must stay JSON, not HTML: {ct_rpc:?}"
    );
}

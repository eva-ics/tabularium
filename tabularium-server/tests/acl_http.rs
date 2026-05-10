//! Stage-1 ACL HTTP smoke — the Emperor demands a keyed gate when `authenticate = true`.

use std::sync::Arc;
use std::time::Duration;

use bma_ts::Monotonic;
use common::spawn_test_server_with_wait_timeout;
use serde_json::json;
use tabularium::SqliteDatabase;
use tabularium::jsonrpc_codes::UNAUTHORIZED;
use tabularium_server::web::{AppState, router};
use tokio::net::TcpListener;

mod common;

#[tokio::test]
async fn rest_requires_x_auth_key_when_authenticate_true() {
    let s = spawn_test_server_with_wait_timeout(Duration::from_secs(3), true).await;
    let c = reqwest::Client::new();
    let r = c
        .get(format!("{}/api/doc", s.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rpc_returns_unauthorized_when_authenticate_true_and_missing_key() {
    let s = spawn_test_server_with_wait_timeout(Duration::from_secs(3), true).await;
    let c = reqwest::Client::new();
    let body = json!({
        "jsonrpc": "2.0",
        "method": "list_directory",
        "params": { "path": "/" },
        "id": 1_i64,
    });
    let r = c
        .post(format!("{}/rpc", s.base_url))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let v: serde_json::Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], UNAUTHORIZED as i64);
}

#[tokio::test]
async fn acl_roundtrip_when_authenticate_false() {
    let s = spawn_test_server_with_wait_timeout(Duration::from_secs(3), false).await;
    let c = reqwest::Client::new();
    let acl_body = r#"{"admin":true,"allow":{"read":[],"write":[]},"deny":{"read":[],"write":[]}}"#;
    let put = json!({
        "jsonrpc": "2.0",
        "method": "acl_put",
        "params": { "name": "adm", "body": acl_body },
        "id": 2_i64,
    });
    let r = c
        .post(format!("{}/rpc", s.base_url))
        .json(&put)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let v: serde_json::Value = r.json().await.unwrap();
    assert!(v.get("error").is_none(), "{v:?}");

    let list = json!({
        "jsonrpc": "2.0",
        "method": "acl_list",
        "params": {},
        "id": 3_i64,
    });
    let r = c
        .post(format!("{}/rpc", s.base_url))
        .json(&list)
        .send()
        .await
        .unwrap();
    let v: serde_json::Value = r.json().await.unwrap();
    let arr = v["result"].as_array().expect("acl_list array");
    assert!(arr.iter().any(|row| row["name"] == "adm"));
}

#[tokio::test]
async fn list_root_filtered_when_acl_allows_only_child_path() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("acl_filter.db");
    let idx_path = dir.path().join("acl_filter.idx");
    let uri = format!("sqlite://{}", db_path.display());
    let db = Arc::new(SqliteDatabase::init(&uri, &idx_path, 8).await.unwrap());
    db.create_directory("/alpha", None, false).await.unwrap();
    db.create_directory("/test", None, false).await.unwrap();
    let acl =
        r#"{"admin":false,"allow":{"read":["/test"],"write":[]},"deny":{"read":[],"write":[]}}"#;
    db.acl_upsert_validated("narrow", acl).await.unwrap();
    db.psk_insert("u1", "narrow", "narrow-test-psk-abcdefghijklmnopqrst")
        .await
        .unwrap();

    let app = router(AppState {
        db: Arc::clone(&db),
        wait_timeout: Duration::from_secs(3),
        process_started_at: Monotonic::now(),
        authenticate_api: true,
        authenticate_mcp: false,
        oidc: None,
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(120)).await;

    let base = format!("http://{}", addr);
    let c = reqwest::Client::new();
    let r = c
        .get(format!("{base}/api/doc"))
        .header("X-Auth-Key", "narrow-test-psk-abcdefghijklmnopqrst")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let arr = r.json::<serde_json::Value>().await.unwrap();
    let rows = arr.as_array().expect("root list array");
    let names: Vec<&str> = rows.iter().filter_map(|row| row["name"].as_str()).collect();
    assert!(names.contains(&"test"));
    assert!(!names.contains(&"alpha"));
}

#[tokio::test]
async fn list_root_shows_dir_when_acl_allows_only_subtree_glob() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("acl_subtree.db");
    let idx_path = dir.path().join("acl_subtree.idx");
    let uri = format!("sqlite://{}", db_path.display());
    let db = Arc::new(SqliteDatabase::init(&uri, &idx_path, 8).await.unwrap());
    db.create_directory("/alpha", None, false).await.unwrap();
    db.create_directory("/beta", None, false).await.unwrap();
    db.create_directory("/beta/nested", None, false)
        .await
        .unwrap();
    let acl =
        r#"{"admin":false,"allow":{"read":["/beta/*"],"write":[]},"deny":{"read":[],"write":[]}}"#;
    db.acl_upsert_validated("sub", acl).await.unwrap();
    db.psk_insert("u2", "sub", "subtree-test-psk-abcdefghijklmnopqrst")
        .await
        .unwrap();

    let app = router(AppState {
        db: Arc::clone(&db),
        wait_timeout: Duration::from_secs(3),
        process_started_at: Monotonic::now(),
        authenticate_api: true,
        authenticate_mcp: false,
        oidc: None,
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(120)).await;

    let base = format!("http://{}", addr);
    let c = reqwest::Client::new();
    let hdr = ("X-Auth-Key", "subtree-test-psk-abcdefghijklmnopqrst");

    let root = c
        .get(format!("{base}/api/doc"))
        .header(hdr.0, hdr.1)
        .send()
        .await
        .unwrap();
    assert_eq!(root.status(), reqwest::StatusCode::OK);
    let rows = root.json::<serde_json::Value>().await.unwrap();
    let names: Vec<&str> = rows
        .as_array()
        .expect("root")
        .iter()
        .filter_map(|row| row["name"].as_str())
        .collect();
    assert!(names.contains(&"beta"));
    assert!(!names.contains(&"alpha"));

    let beta = c
        .get(format!("{base}/api/doc/beta"))
        .header(hdr.0, hdr.1)
        .send()
        .await
        .unwrap();
    assert_eq!(beta.status(), reqwest::StatusCode::OK);
    let beta_rows = beta.json::<serde_json::Value>().await.unwrap();
    let beta_names: Vec<&str> = beta_rows
        .as_array()
        .expect("beta")
        .iter()
        .filter_map(|row| row["name"].as_str())
        .collect();
    assert!(beta_names.contains(&"nested"));
}

#[tokio::test]
async fn rpc_exists_allows_directory_path_when_acl_is_subtree_glob_only() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("acl_exists_tree.db");
    let idx_path = dir.path().join("acl_exists_tree.idx");
    let uri = format!("sqlite://{}", db_path.display());
    let db = Arc::new(SqliteDatabase::init(&uri, &idx_path, 8).await.unwrap());
    db.create_directory("/beta", None, false).await.unwrap();
    db.create_directory("/beta/nested", None, false)
        .await
        .unwrap();
    let acl =
        r#"{"admin":false,"allow":{"read":["/beta/*"],"write":[]},"deny":{"read":[],"write":[]}}"#;
    db.acl_upsert_validated("sub", acl).await.unwrap();
    db.psk_insert("u3", "sub", "exists-tree-psk-abcdefghijklmnopqrst")
        .await
        .unwrap();

    let app = router(AppState {
        db: Arc::clone(&db),
        wait_timeout: Duration::from_secs(3),
        process_started_at: Monotonic::now(),
        authenticate_api: true,
        authenticate_mcp: false,
        oidc: None,
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(120)).await;

    let base = format!("http://{}", addr);
    let c = reqwest::Client::new();
    let hdr = ("X-Auth-Key", "exists-tree-psk-abcdefghijklmnopqrst");

    for path in ["/beta", "/beta/nested"] {
        let body = json!({
            "jsonrpc": "2.0",
            "method": "exists",
            "params": { "path": path },
            "id": 1_i64,
        });
        let r = c
            .post(format!("{base}/rpc"))
            .header(hdr.0, hdr.1)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), reqwest::StatusCode::OK);
        let v: serde_json::Value = r.json().await.unwrap();
        assert!(v.get("error").is_none(), "{path}: {v:?}");
        assert_eq!(v["result"], false, "{path}: directory-only path");
    }
}

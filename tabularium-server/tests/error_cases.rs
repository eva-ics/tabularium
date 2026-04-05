//! HTTP status and JSON-RPC codes for error paths.

mod common;

use common::spawn_test_server;
use serde_json::{Value, json};
use tabularium::jsonrpc_codes::DUPLICATE_RESOURCE;

#[tokio::test]
async fn rest_not_found_and_conflict() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();

    let r = client
        .get(format!("{base}/api/doc/no_such_dir_ever"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NOT_FOUND);

    let r = client
        .get(format!("{base}/api/doc/no_such_dir_ever/doc"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NOT_FOUND);

    client
        .post(format!("{base}/api/doc"))
        .json(&json!({ "path": "/dup_rest_cat", "description": null }))
        .send()
        .await
        .unwrap();

    let r = client
        .post(format!("{base}/api/doc"))
        .json(&json!({ "path": "/dup_rest_cat", "description": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::CONFLICT);

    client
        .post(format!("{base}/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "create_document",
            "params": { "path": "/dup_rest_cat/solo", "content": "a" },
            "id": 1_i64,
        }))
        .send()
        .await
        .unwrap();

    let r = client
        .post(format!("{base}/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "create_document",
            "params": { "path": "/dup_rest_cat/solo", "content": "b" },
            "id": 2_i64,
        }))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], DUPLICATE_RESOURCE);
}

#[tokio::test]
async fn rest_delete_nonempty_directory_is_conflict() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();
    let cat = "rest_nonempty_cat";
    client
        .post(format!("{base}/api/doc"))
        .json(&json!({ "path": format!("/{cat}"), "description": null }))
        .send()
        .await
        .unwrap();
    client
        .put(format!("{base}/api/doc/{cat}/d"))
        .json(&json!({ "content": "z" }))
        .send()
        .await
        .unwrap();

    let r = client
        .delete(format!("{base}/api/doc/{cat}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::CONFLICT);
}

#[tokio::test]
async fn rpc_get_document_missing_returns_minus_32603() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();

    let body = json!({
        "jsonrpc": "2.0",
        "method": "get_document",
        "params": { "path": "/phantom_cat/phantom_doc" },
        "id": 9_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], -32603);
}

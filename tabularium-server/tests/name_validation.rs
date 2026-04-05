//! Invalid names rejected at REST and JSON-RPC.

mod common;

use common::spawn_test_server;
use serde_json::{Value, json};

async fn rpc(client: &reqwest::Client, base: &str, method: &str, params: Value) -> Value {
    let body = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    r.json().await.unwrap()
}

#[tokio::test]
async fn rest_rejects_bad_directory_and_document_names() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();

    let r = client
        .post(format!("{base}/api/doc"))
        .json(&json!({ "path": "bad/name", "description": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST);

    let r = client
        .post(format!("{base}/api/doc"))
        .json(&json!({ "path": r"/bad\slash", "description": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST);

    let r = client
        .post(format!("{base}/api/doc"))
        .json(&json!({ "path": "", "description": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST);

    let r = client
        .post(format!("{base}/api/doc"))
        .json(&json!({ "path": "/42", "description": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST);

    // Valid directory for document path tests
    client
        .post(format!("{base}/api/doc"))
        .json(&json!({ "path": "/valcat", "description": null }))
        .send()
        .await
        .unwrap();

    let r = client
        .put(format!("{base}/api/doc/valcat/999"))
        .json(&json!({ "content": "c" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn rpc_rejects_bad_paths_and_names() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();

    let v = rpc(
        &client,
        base,
        "create_directory",
        json!({ "path": "a/b", "description": null }),
    )
    .await;
    assert_eq!(v["error"]["code"], -32602);

    let v = rpc(
        &client,
        base,
        "create_directory",
        json!({ "path": r"/x\y", "description": null }),
    )
    .await;
    assert_eq!(v["error"]["code"], -32602);

    let v = rpc(
        &client,
        base,
        "create_directory",
        json!({ "path": "", "description": null }),
    )
    .await;
    assert_eq!(v["error"]["code"], -32602);

    let v = rpc(
        &client,
        base,
        "create_directory",
        json!({ "path": "/okcat", "description": null }),
    )
    .await;
    assert!(v.get("result").is_some());

    let v = rpc(
        &client,
        base,
        "create_directory",
        json!({ "path": "/77", "description": null }),
    )
    .await;
    assert_eq!(v["error"]["code"], -32602);

    let v = rpc(
        &client,
        base,
        "create_document",
        json!({ "path": "/okcat/bad//name", "content": "x" }),
    )
    .await;
    assert_eq!(v["error"]["code"], -32602);

    let v = rpc(
        &client,
        base,
        "create_document",
        json!({ "path": "/okcat/42", "content": "x" }),
    )
    .await;
    assert_eq!(v["error"]["code"], -32602);
}

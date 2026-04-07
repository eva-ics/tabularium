//! Path normalization corpus across multiple RPC methods.

mod common;

use common::spawn_test_server;
use reqwest::Client;
use serde_json::{Value, json};

async fn rpc_raw(base: &str, method: &str, params: Value) -> Value {
    let http = Client::new();
    let body = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1_i64,
    });
    http.post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn rpc_ok(base: &str, method: &str, params: Value) -> Value {
    let v = rpc_raw(base, method, params).await;
    assert!(v.get("error").is_none(), "unexpected rpc error: {v:?}");
    v["result"].clone()
}

#[tokio::test]
async fn test_rpc_accepts_unrooted_path() {
    let s = spawn_test_server().await;
    let base = &s.base_url;

    // mkdir (parents) with unrooted multi-segment path.
    rpc_ok(
        base,
        "create_directory",
        json!({ "path": "a/b", "parents": true }),
    )
    .await;

    // create_document with unrooted path.
    rpc_ok(
        base,
        "create_document",
        json!({ "path": "a/b/doc", "content": "x" }),
    )
    .await;

    // exists accepts unrooted.
    let ex = rpc_ok(base, "exists", json!({ "path": "a/b/doc" })).await;
    assert_eq!(ex, Value::Bool(true));

    // stat accepts unrooted.
    let st = rpc_ok(base, "stat", json!({ "path": "a/b/doc" })).await;
    assert_eq!(st["path"], "/a/b/doc");

    // wc accepts unrooted.
    let wc = rpc_ok(base, "wc", json!({ "path": "a/b/doc" })).await;
    assert!(wc.get("bytes").is_some());
}

#[tokio::test]
async fn test_rpc_rejects_empty_path() {
    let s = spawn_test_server().await;
    let base = &s.base_url;

    let v = rpc_raw(base, "get_document_ref", json!({ "path": "" })).await;
    assert!(v.get("result").is_none());
    assert!(v.get("error").is_some());
}

#[tokio::test]
async fn test_rpc_normalizes_double_slash() {
    let s = spawn_test_server().await;
    let base = &s.base_url;

    rpc_ok(
        base,
        "create_directory",
        json!({ "path": "/aa/bb", "parents": true }),
    )
    .await;
    rpc_ok(
        base,
        "create_document",
        json!({ "path": "/aa/bb/doc", "content": "y" }),
    )
    .await;

    let r = rpc_ok(base, "get_document_ref", json!({ "path": "aa//bb//doc" })).await;
    assert_eq!(r["path"], "/aa/bb/doc");
}

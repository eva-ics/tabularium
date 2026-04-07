//! Server-side RPC path normalization regression tests.

mod common;

use common::spawn_test_server;
use reqwest::Client;
use serde_json::{Value, json};

async fn rpc(base: &str, method: &str, params: serde_json::Value) -> Value {
    let http = Client::new();
    let body = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1_i64,
    });
    let r = http
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let v: Value = r.json().await.unwrap();
    assert!(v.get("error").is_none(), "unexpected rpc error: {v:?}");
    v["result"].clone()
}

#[tokio::test]
async fn rpc_accepts_unrooted_and_normalizes_slashes_and_dots() {
    let s = spawn_test_server().await;
    let base = &s.base_url;

    rpc(
        base,
        "create_directory",
        json!({ "path": "rpc//a/./b", "parents": true }),
    )
    .await;

    rpc(
        base,
        "create_document",
        json!({ "path": "rpc/a/b/./doc", "content": "x" }),
    )
    .await;

    let got = rpc(base, "get_document_ref", json!({ "path": "rpc/a//b/doc" })).await;
    assert_eq!(got["path"], "/rpc/a/b/doc");
    assert_eq!(got["name"], "doc");
}

#[tokio::test]
async fn rpc_resolves_dotdot_without_escaping_root() {
    let s = spawn_test_server().await;
    let base = &s.base_url;

    rpc(
        base,
        "create_directory",
        json!({ "path": "/rpc2/a/b", "parents": true }),
    )
    .await;
    rpc(
        base,
        "create_document",
        json!({ "path": "/rpc2/a/b/doc", "content": "y" }),
    )
    .await;

    let got = rpc(
        base,
        "get_document_ref",
        json!({ "path": "rpc2/a/b/../b/doc" }),
    )
    .await;
    assert_eq!(got["path"], "/rpc2/a/b/doc");
    assert_eq!(got["name"], "doc");
}

#[tokio::test]
async fn rpc_rejects_escape_above_root() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let http = Client::new();
    let body = json!({
        "jsonrpc": "2.0",
        "method": "list_directory",
        "params": { "path": "/../a" },
        "id": 1_i64,
    });
    let r = http
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let v: Value = r.json().await.unwrap();
    assert!(v.get("result").is_none());
    assert!(v.get("error").is_some());
}

#[tokio::test]
async fn copy_entries_rejects_copy_dir_into_itself() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let http = Client::new();

    // Create /xxx/chats and one doc.
    let mkdir = json!({
        "jsonrpc": "2.0",
        "method": "create_directory",
        "params": { "path": "/xxx/chats", "parents": true },
        "id": 1_i64,
    });
    http.post(format!("{base}/rpc"))
        .json(&mkdir)
        .send()
        .await
        .unwrap();

    rpc(
        base,
        "create_document",
        json!({ "path": "/xxx/chats/chat1", "content": "x" }),
    )
    .await;

    // Attempt to copy /xxx/chats into /xxx (which resolves to /xxx/chats).
    let body = json!({
        "jsonrpc": "2.0",
        "method": "copy_entries",
        "params": { "src": "/xxx/chats", "dst": "/xxx", "recursive": true },
        "id": 2_i64,
    });
    let r = http
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("result").is_none());
    assert!(v.get("error").is_some());
}

#[tokio::test]
async fn test_cp_rpc_alias_cp_rejected() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let http = Client::new();
    let body = json!({
        "jsonrpc": "2.0",
        "method": "cp",
        "params": { "src": "/a", "dst": "/b", "recursive": true },
        "id": 1_i64,
    });
    let r = http
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("result").is_none());
    assert_eq!(v["error"]["code"], -32601);
}

#[tokio::test]
async fn test_copy_entries_dst_root_slash_not_empty_error() {
    let s = spawn_test_server().await;
    let base = &s.base_url;

    // Use put_document so parent directories are created implicitly.
    rpc(
        base,
        "put_document",
        json!({ "path": "src/file", "content": "x" }),
    )
    .await;

    // Copy file into root dir should create /file.
    rpc(
        base,
        "copy_entries",
        json!({ "src": "src/file", "dst": "/", "recursive": false }),
    )
    .await;
    let got = rpc(base, "get_document", json!({ "path": "/file" })).await;
    assert_eq!(got["content"], "x");
}

#[tokio::test]
async fn test_cp_self_copy_rejected() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let http = Client::new();

    rpc(
        base,
        "create_directory",
        json!({ "path": "/self", "parents": true }),
    )
    .await;
    rpc(
        base,
        "create_document",
        json!({ "path": "/self/f", "content": "x" }),
    )
    .await;

    let body = json!({
        "jsonrpc": "2.0",
        "method": "copy_entries",
        "params": { "src": "/self/f", "dst": "/self/f", "recursive": false },
        "id": 2_i64,
    });
    let r = http
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("result").is_none());
    assert!(v.get("error").is_some());
}

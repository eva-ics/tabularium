//! `force` guard for `put_document` and `append_document` (JSON-RPC).
//!
//! Default safe (`force` omitted or `false`): existing target → `Duplicate` (-32002).
//! `force = true`: legacy upsert (put replaces, append appends).
//! REST `PUT` / `PATCH` keep HTTP upsert convention regardless of `force`.
//!
//! *In the Emperor's name, no silent overwrite.*

mod common;

use common::spawn_test_server;
use serde_json::{Value, json};
use tabularium::jsonrpc_codes::DUPLICATE_RESOURCE;

async fn rpc(base: &str, method: &str, params: Value) -> Value {
    let body = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1_i64,
    });
    let r = reqwest::Client::new()
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    r.json().await.unwrap()
}

async fn ensure_dir(base: &str, name: &str) {
    let _ = rpc(
        base,
        "create_directory",
        json!({ "path": format!("/{name}"), "description": null }),
    )
    .await;
}

#[tokio::test]
async fn put_document_default_force_false_creates_then_rejects_existing() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    ensure_dir(base, "fg_put_default").await;

    // First call (omitted force) creates.
    let v = rpc(
        base,
        "put_document",
        json!({ "path": "/fg_put_default/d", "content": "first" }),
    )
    .await;
    assert!(v.get("result").is_some(), "first put: {v}");

    // Second call (omitted force) on the same path → Duplicate.
    let v = rpc(
        base,
        "put_document",
        json!({ "path": "/fg_put_default/d", "content": "second" }),
    )
    .await;
    assert_eq!(v["error"]["code"], DUPLICATE_RESOURCE, "second put: {v}");

    // Body must remain "first" (no silent overwrite).
    let v = rpc(base, "get_document", json!({ "path": "/fg_put_default/d" })).await;
    assert_eq!(v["result"]["content"], "first");
}

#[tokio::test]
async fn put_document_force_true_overwrites() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    ensure_dir(base, "fg_put_force").await;

    rpc(
        base,
        "put_document",
        json!({ "path": "/fg_put_force/d", "content": "first" }),
    )
    .await;

    let v = rpc(
        base,
        "put_document",
        json!({ "path": "/fg_put_force/d", "content": "OVERWRITE", "force": true }),
    )
    .await;
    assert!(v.get("result").is_some(), "force-true put: {v}");

    let v = rpc(base, "get_document", json!({ "path": "/fg_put_force/d" })).await;
    assert_eq!(v["result"]["content"], "OVERWRITE");
}

#[tokio::test]
async fn append_document_default_force_false_creates_then_rejects_existing() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    ensure_dir(base, "fg_app_default").await;

    let v = rpc(
        base,
        "append_document",
        json!({ "path": "/fg_app_default/d", "content": "alpha" }),
    )
    .await;
    assert!(v.get("result").is_some(), "first append: {v}");

    let v = rpc(
        base,
        "append_document",
        json!({ "path": "/fg_app_default/d", "content": "beta" }),
    )
    .await;
    assert_eq!(v["error"]["code"], DUPLICATE_RESOURCE, "second append: {v}");

    let v = rpc(base, "get_document", json!({ "path": "/fg_app_default/d" })).await;
    assert_eq!(v["result"]["content"], "alpha");
}

#[tokio::test]
async fn append_document_force_true_appends_not_replaces() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    ensure_dir(base, "fg_app_force").await;

    rpc(
        base,
        "put_document",
        json!({ "path": "/fg_app_force/d", "content": "head" }),
    )
    .await;

    let v = rpc(
        base,
        "append_document",
        json!({ "path": "/fg_app_force/d", "content": "tail", "force": true }),
    )
    .await;
    assert!(v.get("result").is_some(), "force-true append: {v}");

    // append() inserts a single newline boundary when needed; the head was "head" (no newline).
    let v = rpc(base, "get_document", json!({ "path": "/fg_app_force/d" })).await;
    assert_eq!(v["result"]["content"], "head\ntail");
}

#[tokio::test]
async fn force_string_value_is_accepted() {
    // Some clients send "true"/"false" as strings; the optional bool extractor accepts them.
    let s = spawn_test_server().await;
    let base = &s.base_url;
    ensure_dir(base, "fg_str").await;

    rpc(
        base,
        "put_document",
        json!({ "path": "/fg_str/d", "content": "first" }),
    )
    .await;

    let v = rpc(
        base,
        "put_document",
        json!({ "path": "/fg_str/d", "content": "second", "force": "true" }),
    )
    .await;
    assert!(v.get("result").is_some(), "string-true force put: {v}");

    let v = rpc(base, "get_document", json!({ "path": "/fg_str/d" })).await;
    assert_eq!(v["result"]["content"], "second");
}

#[tokio::test]
async fn say_document_remains_exempt_missing_target_errors() {
    // Regression guard for the documented exemption: say_document still cannot
    // create new scrolls; it errors when the target is missing.
    let s = spawn_test_server().await;
    let base = &s.base_url;
    ensure_dir(base, "fg_say_exempt").await;

    let v = rpc(
        base,
        "say_document",
        json!({
            "path": "/fg_say_exempt/no_such_doc",
            "from_id": "Cogis",
            "content": "hello",
        }),
    )
    .await;
    assert!(v.get("error").is_some(), "say should error: {v}");
}

#[tokio::test]
async fn rest_put_keeps_upsert_convention_unaffected_by_force_guard() {
    // HTTP PUT is upsert by convention; the force guard is JSON-RPC / MCP only.
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();
    let cat = "fg_rest_put";
    ensure_dir(base, cat).await;

    // First PUT — creates.
    let r = client
        .put(format!("{base}/api/doc/{cat}/d"))
        .json(&json!({ "content": "first" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);

    // Second PUT — replaces (no Duplicate).
    let r = client
        .put(format!("{base}/api/doc/{cat}/d"))
        .json(&json!({ "content": "second" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);

    let r = client
        .get(format!("{base}/api/doc/{cat}/d"))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["content"], "second");
}

#[tokio::test]
async fn put_document_concurrent_force_false_one_winner_others_duplicate() {
    // Atomicity proof: many concurrent agents racing to create the same path
    // must produce exactly one Ok and N-1 Duplicate. No silent corruption.
    let s = spawn_test_server().await;
    let base = std::sync::Arc::new(s.base_url.clone());
    ensure_dir(&base, "fg_race").await;

    let path = "/fg_race/contested";
    let n: usize = 8;
    let mut handles = Vec::with_capacity(n);
    for i in 0..n {
        let base = std::sync::Arc::clone(&base);
        let payload = format!("agent-{i}");
        handles.push(tokio::spawn(async move {
            rpc(
                &base,
                "put_document",
                json!({ "path": path, "content": payload }),
            )
            .await
        }));
    }

    let mut ok_count = 0;
    let mut dup_count = 0;
    for h in handles {
        let v = h.await.unwrap();
        if v.get("result").is_some() {
            ok_count += 1;
        } else if v["error"]["code"].as_i64() == Some(i64::from(DUPLICATE_RESOURCE)) {
            dup_count += 1;
        } else {
            panic!("unexpected RPC outcome: {v}");
        }
    }
    assert_eq!(ok_count, 1, "exactly one creator must win the race");
    assert_eq!(dup_count, n - 1, "all losers must see Duplicate");

    let v = rpc(&base, "get_document", json!({ "path": path })).await;
    assert!(
        v["result"]["content"]
            .as_str()
            .unwrap()
            .starts_with("agent-"),
        "got body {v}"
    );
}

//! Document `revision` (UUID v4) and `only_if_revision` compare-and-swap — Ferrum matrix coverage.

mod common;

use common::spawn_test_server;
use serde_json::{Value, json};
use tabularium::jsonrpc_codes::{DUPLICATE_RESOURCE, REVISION_MISMATCH};
use uuid::Uuid;

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

#[tokio::test]
async fn get_document_leaves_revision_stable_across_reads() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    rpc(
        base,
        "create_directory",
        json!({ "path": "/rev_read", "parents": true }),
    )
    .await;
    let v = rpc(
        base,
        "create_document",
        json!({ "path": "/rev_read/doc", "content": "alpha" }),
    )
    .await;
    assert!(v.get("error").is_none(), "{v:?}");
    let rev0 = v["result"]["revision"].as_str().unwrap();
    Uuid::parse_str(rev0).unwrap();

    let g1 = rpc(base, "get_document", json!({ "path": "/rev_read/doc" })).await;
    assert!(g1.get("error").is_none(), "{g1:?}");
    assert_eq!(g1["result"]["revision"].as_str().unwrap(), rev0);

    let g2 = rpc(base, "get_document", json!({ "path": "/rev_read/doc" })).await;
    assert_eq!(g2["result"]["revision"].as_str().unwrap(), rev0);
}

#[tokio::test]
async fn put_document_only_if_revision_errors_and_success() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    rpc(
        base,
        "create_directory",
        json!({ "path": "/cas_put", "parents": true }),
    )
    .await;

    let miss = rpc(
        base,
        "put_document",
        json!({
            "path": "/cas_put/missing",
            "content": "x",
            "only_if_revision": Uuid::nil().to_string(),
        }),
    )
    .await;
    assert_eq!(miss["error"]["code"], -32603);

    rpc(
        base,
        "put_document",
        json!({ "path": "/cas_put/a", "content": "one" }),
    )
    .await;

    let rev =
        rpc(base, "get_document", json!({ "path": "/cas_put/a" })).await["result"]["revision"]
            .as_str()
            .unwrap()
            .to_string();

    let bad = rpc(
        base,
        "put_document",
        json!({
            "path": "/cas_put/a",
            "content": "two",
            "force": true,
            "only_if_revision": Uuid::nil().to_string(),
        }),
    )
    .await;
    assert_eq!(bad["error"]["code"], REVISION_MISMATCH);

    let ok = rpc(
        base,
        "put_document",
        json!({
            "path": "/cas_put/a",
            "content": "three",
            "force": false,
            "only_if_revision": rev,
        }),
    )
    .await;
    assert!(ok.get("error").is_none(), "{ok:?}");
    assert_ne!(ok["result"]["revision"].as_str().unwrap(), rev.as_str());

    let body = rpc(base, "get_document", json!({ "path": "/cas_put/a" })).await;
    assert_eq!(body["result"]["content"], "three");
}

#[tokio::test]
async fn create_document_only_if_revision_matrix() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    rpc(
        base,
        "create_directory",
        json!({ "path": "/cas_cr", "parents": true }),
    )
    .await;

    let miss = rpc(
        base,
        "create_document",
        json!({
            "path": "/cas_cr/new",
            "content": "a",
            "only_if_revision": Uuid::nil().to_string(),
        }),
    )
    .await;
    assert_eq!(miss["error"]["code"], -32603);

    rpc(
        base,
        "create_document",
        json!({ "path": "/cas_cr/new", "content": "first" }),
    )
    .await;

    let dup = rpc(
        base,
        "create_document",
        json!({ "path": "/cas_cr/new", "content": "nope" }),
    )
    .await;
    assert_eq!(dup["error"]["code"], DUPLICATE_RESOURCE);

    let rev =
        rpc(base, "get_document", json!({ "path": "/cas_cr/new" })).await["result"]["revision"]
            .as_str()
            .unwrap()
            .to_string();

    let mis = rpc(
        base,
        "create_document",
        json!({
            "path": "/cas_cr/new",
            "content": "second",
            "force": true,
            "only_if_revision": Uuid::nil().to_string(),
        }),
    )
    .await;
    assert_eq!(mis["error"]["code"], REVISION_MISMATCH);

    let ok = rpc(
        base,
        "create_document",
        json!({
            "path": "/cas_cr/new",
            "content": "third",
            "force": true,
            "only_if_revision": rev,
        }),
    )
    .await;
    assert!(ok.get("error").is_none(), "{ok:?}");

    let body = rpc(base, "get_document", json!({ "path": "/cas_cr/new" })).await;
    assert_eq!(body["result"]["content"], "third");
}

#[tokio::test]
async fn list_directory_revision_null_for_directories() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    rpc(
        base,
        "create_directory",
        json!({ "path": "/mix/sub", "parents": true }),
    )
    .await;
    rpc(
        base,
        "create_document",
        json!({ "path": "/mix/a", "content": "z" }),
    )
    .await;

    let v = rpc(base, "list_directory", json!({ "path": "/mix" })).await;
    assert!(v.get("error").is_none(), "{v:?}");
    let arr = v["result"].as_array().unwrap();
    let file = arr.iter().find(|r| r["name"] == "a").expect("file row");
    let dir = arr.iter().find(|r| r["name"] == "sub").expect("dir row");
    assert!(file["revision"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(dir["revision"].is_null());
}

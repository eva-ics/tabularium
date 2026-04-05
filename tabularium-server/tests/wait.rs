//! Long-polling wait tests: REST ?wait=true and RPC `wait` method.

mod common;

use std::time::Duration;

use common::{spawn_test_server, spawn_test_server_with_wait_timeout};
use reqwest::Client;
use serde_json::{Value, json};

#[tokio::test]
async fn rest_wait_returns_204_on_write() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let cat = "wait_cat";
    client
        .post(format!("{base}/api/doc"))
        .json(&json!({ "path": format!("/{cat}"), "description": null }))
        .send()
        .await
        .unwrap();
    client
        .put(format!("{base}/api/doc/{cat}/wdoc"))
        .json(&json!({ "content": "v0" }))
        .send()
        .await
        .unwrap();

    let url = format!("{base}/api/doc/{cat}/wdoc?wait=true");
    let h = tokio::spawn(async move { client.get(&url).send().await.unwrap() });
    tokio::time::sleep(Duration::from_millis(60)).await;

    let client2 = Client::new();
    let r = client2
        .patch(format!("{base}/api/doc/{cat}/wdoc"))
        .json(&json!({ "content": "v1" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);

    let wr = h.await.unwrap();
    assert_eq!(wr.status(), reqwest::StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn rest_wait_times_out_504() {
    let s = spawn_test_server_with_wait_timeout(Duration::from_millis(400)).await;
    let base = &s.base_url;
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let cat = "wait_timeout_cat";
    client
        .post(format!("{base}/api/doc"))
        .json(&json!({ "path": format!("/{cat}"), "description": null }))
        .send()
        .await
        .unwrap();
    client
        .put(format!("{base}/api/doc/{cat}/tdoc"))
        .json(&json!({ "content": "still" }))
        .send()
        .await
        .unwrap();

    let r = client
        .get(format!("{base}/api/doc/{cat}/tdoc?wait=true"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::GATEWAY_TIMEOUT);
}

#[tokio::test]
async fn rest_wait_missing_document_is_404() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = Client::new();
    let r = client
        .get(format!(
            "{base}/api/doc/absolutely_no_such_cat_zzz/missing?wait=true"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rpc_wait_returns_null_on_change() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let cat = "rpc_wait_cat";
    let path = format!("/{cat}/rdoc");
    client
        .post(format!("{base}/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "create_directory",
            "params": { "path": format!("/{cat}"), "description": null },
            "id": 1_i64,
        }))
        .send()
        .await
        .unwrap();
    client
        .post(format!("{base}/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "create_document",
            "params": { "path": path, "content": "a" },
            "id": 2_i64,
        }))
        .send()
        .await
        .unwrap();

    let h = tokio::spawn({
        let base = base.to_string();
        let path = path.clone();
        async move {
            let c = Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap();
            c.post(format!("{base}/rpc"))
                .json(&json!({
                    "jsonrpc": "2.0",
                    "method": "wait",
                    "params": { "path": path },
                    "id": 99_i64,
                }))
                .send()
                .await
                .unwrap()
        }
    });
    tokio::time::sleep(Duration::from_millis(60)).await;

    let client2 = Client::new();
    let r = client2
        .post(format!("{base}/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "update_document",
            "params": { "path": path, "content": "b" },
            "id": 3_i64,
        }))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("result").is_some());

    let wr = h.await.unwrap();
    let v2: Value = wr.json().await.unwrap();
    assert_eq!(v2["result"], Value::Null);
}

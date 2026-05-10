//! Stage-1 ACL HTTP smoke — the Emperor demands a keyed gate when `authenticate = true`.

use std::time::Duration;

use common::spawn_test_server_with_wait_timeout;
use serde_json::json;
use tabularium::jsonrpc_codes::UNAUTHORIZED;

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

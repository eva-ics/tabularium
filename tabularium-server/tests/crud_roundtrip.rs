//! REST and RPC create / read / update / append / delete round-trips.

mod common;

use common::spawn_test_server;
use reqwest::header::LOCATION;
use serde_json::{Value, json};

#[tokio::test]
async fn rest_patch_creates_missing_document() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();
    let cat = "append_create_cat";
    client
        .post(format!("{base}/api/doc"))
        .json(&json!({ "path": format!("/{cat}"), "description": null }))
        .send()
        .await
        .unwrap();

    let r = client
        .patch(format!("{base}/api/doc/{cat}/brand_new_patch_doc"))
        .json(&json!({ "content": "first bytes" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);

    let r = client
        .get(format!("{base}/api/doc/{cat}/brand_new_patch_doc"))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["content"], "first bytes");
}

#[tokio::test]
async fn rest_patch_no_double_newline_when_body_ends_with_newline() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();
    let cat = "nl_cat";
    client
        .post(format!("{base}/api/doc"))
        .json(&json!({ "path": format!("/{cat}"), "description": null }))
        .send()
        .await
        .unwrap();
    client
        .put(format!("{base}/api/doc/{cat}/d"))
        .json(&json!({ "content": "line\n" }))
        .send()
        .await
        .unwrap();

    let r = client
        .patch(format!("{base}/api/doc/{cat}/d"))
        .json(&json!({ "content": "next" }))
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
    assert_eq!(v["content"], "line\nnext");
}

#[tokio::test]
async fn rest_patch_empty_body_is_noop_for_existing_document() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();
    let cat = "empty_patch_cat";
    client
        .post(format!("{base}/api/doc"))
        .json(&json!({ "path": format!("/{cat}"), "description": null }))
        .send()
        .await
        .unwrap();
    client
        .put(format!("{base}/api/doc/{cat}/d"))
        .json(&json!({ "content": "line" }))
        .send()
        .await
        .unwrap();

    let r = client
        .patch(format!("{base}/api/doc/{cat}/d"))
        .json(&json!({ "content": "" }))
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
    assert_eq!(v["content"], "line");
}

#[tokio::test]
async fn rpc_delete_directory_recursive_empties_directory() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();
    let cat = "rec_del_cat";
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
    let path = format!("/{cat}/inner");
    client
        .post(format!("{base}/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "create_document",
            "params": { "path": path, "content": "x" },
            "id": 2_i64,
        }))
        .send()
        .await
        .unwrap();

    let r = client
        .post(format!("{base}/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "delete_directory",
            "params": { "path": format!("/{cat}"), "recursive": true },
            "id": 3_i64,
        }))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("result").is_some());

    let r = client.get(format!("{base}/api/doc")).send().await.unwrap();
    let arr: Vec<Value> = r.json().await.unwrap();
    assert!(!arr.iter().any(|c| c["name"] == cat));
}

#[tokio::test]
async fn rest_crud_roundtrip_and_locations() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();

    let cat = "crud_cat";
    let doc = "crud_doc";

    let r = client
        .post(format!("{base}/api/doc"))
        .json(&json!({ "path": format!("/{cat}"), "description": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::CREATED);
    let loc = r.headers().get(LOCATION).unwrap().to_str().unwrap();
    assert!(loc.ends_with(&format!("/api/doc/{cat}")) || loc.contains("crud_cat"));
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["path"], format!("/{cat}"));

    let initial = "first body";
    let r = client
        .put(format!("{base}/api/doc/{cat}/{doc}"))
        .json(&json!({ "content": initial }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);

    let r = client
        .get(format!("{base}/api/doc/{cat}/{doc}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["content"], initial);
    assert_eq!(
        usize::try_from(body["size_bytes"].as_i64().unwrap()).unwrap(),
        initial.len()
    );

    let replaced = "totally replaced";
    let r = client
        .put(format!("{base}/api/doc/{cat}/{doc}"))
        .json(&json!({ "content": replaced }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);

    let r = client
        .get(format!("{base}/api/doc/{cat}/{doc}"))
        .send()
        .await
        .unwrap();
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["content"], replaced);

    let appended = "second line";
    let r = client
        .patch(format!("{base}/api/doc/{cat}/{doc}"))
        .json(&json!({ "content": appended }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);

    let expect = format!("{replaced}\n{appended}");
    let r = client
        .get(format!("{base}/api/doc/{cat}/{doc}"))
        .send()
        .await
        .unwrap();
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["content"], expect);
    assert_eq!(
        usize::try_from(body["size_bytes"].as_i64().unwrap()).unwrap(),
        expect.len()
    );

    let r = client
        .delete(format!("{base}/api/doc/{cat}/{doc}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);

    let r = client
        .get(format!("{base}/api/doc/{cat}/{doc}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn rpc_crud_roundtrip() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();
    let cat = "rpc_cat";
    let path = format!("/{cat}/rpc_note");

    let body = json!({
        "jsonrpc": "2.0",
        "method": "create_directory",
        "params": { "path": format!("/{cat}"), "description": null },
        "id": 1_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("result").is_some());

    let body = json!({
        "jsonrpc": "2.0",
        "method": "create_document",
        "params": { "path": path, "content": "rpc alpha" },
        "id": 2_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("result").is_some());

    let body = json!({
        "jsonrpc": "2.0",
        "method": "get_document",
        "params": { "path": path },
        "id": 3_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["result"]["content"], "rpc alpha");

    let body = json!({
        "jsonrpc": "2.0",
        "method": "update_document",
        "params": { "path": path, "content": "rpc beta" },
        "id": 4_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("result").is_some());

    let body = json!({
        "jsonrpc": "2.0",
        "method": "put_document",
        "params": { "path": path, "content": "\n\n" },
        "id": 5_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("result").is_some());

    let body = json!({
        "jsonrpc": "2.0",
        "method": "get_document",
        "params": { "path": path },
        "id": 6_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["result"]["content"], "");

    let body = json!({
        "jsonrpc": "2.0",
        "method": "append_document",
        "params": { "path": path, "content": "tail" },
        "id": 7_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("result").is_some());

    let body = json!({
        "jsonrpc": "2.0",
        "method": "get_document",
        "params": { "path": path },
        "id": 8_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["result"]["content"], "\ntail");

    let body = json!({
        "jsonrpc": "2.0",
        "method": "delete_document",
        "params": { "path": path },
        "id": 9_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("result").is_some());
}

#[tokio::test]
async fn rpc_say_document() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();
    let cat = "say_rpc_cat";
    let path = format!("/{cat}/say_doc");

    let body = json!({
        "jsonrpc": "2.0",
        "method": "create_directory",
        "params": { "path": format!("/{cat}"), "description": null },
        "id": 1_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("result").is_some());

    let body = json!({
        "jsonrpc": "2.0",
        "method": "create_document",
        "params": { "path": path, "content": "x\n" },
        "id": 2_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("result").is_some());

    let body = json!({
        "jsonrpc": "2.0",
        "method": "say_document",
        "params": { "path": path, "from_id": "ada", "content": "hello" },
        "id": 3_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("result").is_some());

    let body = json!({
        "jsonrpc": "2.0",
        "method": "get_document",
        "params": { "path": path },
        "id": 4_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["result"]["content"], "x\n\n## ada\n\nhello\n\n");
}

#[tokio::test]
async fn rpc_say_document_rejects_missing_target() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();
    let cat = "say_miss_cat";
    let path = format!("/{cat}/ghost");

    let body = json!({
        "jsonrpc": "2.0",
        "method": "create_directory",
        "params": { "path": format!("/{cat}"), "description": null },
        "id": 1_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("result").is_some());

    let body = json!({
        "jsonrpc": "2.0",
        "method": "say_document",
        "params": { "path": path, "from_id": "ada", "content": "nope" },
        "id": 2_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    let err = v["error"]["message"].as_str().unwrap();
    assert!(
        err.contains("say_document") && err.contains("does not exist"),
        "{err}"
    );
}

#[tokio::test]
async fn rpc_touch_document_creates_and_updates_mtime() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();
    let cat = "touch_rpc_cat";
    let path = format!("/{cat}/touched");

    let body = json!({
        "jsonrpc": "2.0",
        "method": "create_directory",
        "params": { "path": format!("/{cat}"), "description": null },
        "id": 1_i64,
    });
    client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();

    let body = json!({
        "jsonrpc": "2.0",
        "method": "touch_document",
        "params": { "path": path },
        "id": 2_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("result").is_some());

    let body = json!({
        "jsonrpc": "2.0",
        "method": "get_document_ref",
        "params": { "path": path },
        "id": 3_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    let m0 = v["result"]["modified_at"].as_u64().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(40)).await;

    let body = json!({
        "jsonrpc": "2.0",
        "method": "touch_document",
        "params": { "path": path },
        "id": 4_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("result").is_some());

    let body = json!({
        "jsonrpc": "2.0",
        "method": "get_document_ref",
        "params": { "path": path },
        "id": 5_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    let m1 = v["result"]["modified_at"].as_u64().unwrap();
    assert!(m1 > m0);
}

#[tokio::test]
async fn rpc_touch_document_sets_exact_timestamp() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();
    let cat = "smtime_rpc_cat";
    let path = format!("/{cat}/pinned");

    let body = json!({
        "jsonrpc": "2.0",
        "method": "create_directory",
        "params": { "path": format!("/{cat}"), "description": null },
        "id": 1_i64,
    });
    client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();

    let body = json!({
        "jsonrpc": "2.0",
        "method": "create_document",
        "params": { "path": path.clone(), "content": "x" },
        "id": 2_i64,
    });
    client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();

    let want_ns = 1_700_000_000_000_000_000_u64;
    let body = json!({
        "jsonrpc": "2.0",
        "method": "touch_document",
        "params": { "path": path.clone(), "modified_at": want_ns },
        "id": 3_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("result").is_some(), "{v:?}");

    let body = json!({
        "jsonrpc": "2.0",
        "method": "get_document_ref",
        "params": { "path": path },
        "id": 4_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["result"]["modified_at"].as_u64().unwrap(), want_ns);
}

#[tokio::test]
async fn rpc_delete_nonempty_directory_fails() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();
    let cat = "nonempty_del_cat";

    let body = json!({
        "jsonrpc": "2.0",
        "method": "create_directory",
        "params": { "path": format!("/{cat}"), "description": null },
        "id": 1_i64,
    });
    client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();

    let path = format!("/{cat}/holds_doc");
    let body = json!({
        "jsonrpc": "2.0",
        "method": "create_document",
        "params": { "path": path, "content": "x" },
        "id": 2_i64,
    });
    client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();

    let body = json!({
        "jsonrpc": "2.0",
        "method": "delete_directory",
        "params": { "path": format!("/{cat}") },
        "id": 3_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("error").is_some());
    assert_eq!(v["error"]["code"], -32602);
}

#[tokio::test]
async fn rpc_describe_get_set_clear_on_directory_and_file() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();
    let cat = "describe_rpc_cat";

    let body = json!({
        "jsonrpc": "2.0",
        "method": "create_directory",
        "params": { "path": format!("/{cat}"), "description": "dir note" },
        "id": 1_i64,
    });
    client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();

    let body = json!({
        "jsonrpc": "2.0",
        "method": "describe",
        "params": { "path": format!("/{cat}") },
        "id": 2_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["result"]["description"], "dir note");

    let path = format!("/{cat}/f");
    let body = json!({
        "jsonrpc": "2.0",
        "method": "create_document",
        "params": { "path": path, "content": "x" },
        "id": 3_i64,
    });
    client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();

    let body = json!({
        "jsonrpc": "2.0",
        "method": "describe",
        "params": { "path": path, "description": "file note" },
        "id": 4_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("error").is_none());
    assert!(v["result"].is_null());

    let body = json!({
        "jsonrpc": "2.0",
        "method": "describe",
        "params": { "path": path },
        "id": 5_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["result"]["description"], "file note");

    let body = json!({
        "jsonrpc": "2.0",
        "method": "describe",
        "params": { "path": path, "description": "" },
        "id": 6_i64,
    });
    client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();

    let body = json!({
        "jsonrpc": "2.0",
        "method": "describe",
        "params": { "path": path },
        "id": 7_i64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v["result"]["description"].is_null());
}

//! REST/RPC proof for deep paths and search `dir` scope (Ferrum §12–18).

mod common;

use common::spawn_test_server;
use serde_json::{Value, json};

#[tokio::test]
async fn rest_deep_mkdir_and_list_chain() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let c = reqwest::Client::new();
    for p in ["/l1", "/l1/l2", "/l1/l2/l3"] {
        let r = c
            .post(format!("{base}/api/doc"))
            .json(&json!({ "path": p, "description": null }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), reqwest::StatusCode::CREATED, "mkdir {p}");
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["path"], p);
    }

    for (path, expect_child) in [
        ("l1/l2/l3", "[]"),
        ("l1/l2", "[\"l3\"]"),
        ("l1", "[\"l2\"]"),
    ] {
        let r = c
            .get(format!("{base}/api/doc/{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), reqwest::StatusCode::OK);
        let j: Value = r.json().await.unwrap();
        let names: Vec<String> = j
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect();
        let names_dbg = format!("{names:?}");
        match expect_child {
            "[]" => assert!(names.is_empty(), "{names_dbg}"),
            "[\"l3\"]" => assert_eq!(names, vec!["l3"], "{names_dbg}"),
            "[\"l2\"]" => assert_eq!(names, vec!["l2"], "{names_dbg}"),
            _ => unreachable!(),
        }
    }
}

#[tokio::test]
async fn rest_root_file_crud() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let c = reqwest::Client::new();
    let r = c
        .put(format!("{base}/api/doc/root_file.txt"))
        .json(&json!({ "content": "hi-root" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);
    let r = c
        .get(format!("{base}/api/doc/root_file.txt"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["content"], "hi-root");
    let r = c.get(format!("{base}/api/doc")).send().await.unwrap();
    let list: Vec<Value> = r.json().await.unwrap();
    let names: Vec<_> = list.iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"root_file.txt"));
}

#[tokio::test]
async fn rest_deep_file_put_patch_delete() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let c = reqwest::Client::new();
    for p in [
        "/deep",
        "/deep/chain",
        "/deep/chain/of",
        "/deep/chain/of/dirs",
    ] {
        c.post(format!("{base}/api/doc"))
            .json(&json!({ "path": p, "description": null }))
            .send()
            .await
            .unwrap();
    }
    let r = c
        .put(format!("{base}/api/doc/deep/chain/of/dirs/note"))
        .json(&json!({ "content": "x0" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);
    let r = c
        .get(format!("{base}/api/doc/deep/chain/of/dirs/note"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.json::<Value>().await.unwrap()["content"], "x0");
    c.patch(format!("{base}/api/doc/deep/chain/of/dirs/note"))
        .json(&json!({ "content": "+tail" }))
        .send()
        .await
        .unwrap();
    let r = c
        .get(format!("{base}/api/doc/deep/chain/of/dirs/note"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.json::<Value>().await.unwrap()["content"], "x0\n+tail");
    c.delete(format!("{base}/api/doc/deep/chain/of/dirs/note"))
        .send()
        .await
        .unwrap();
    let r = c
        .get(format!("{base}/api/doc/deep/chain/of/dirs/note"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rest_delete_nonempty_dir_conflict_then_recursive() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let c = reqwest::Client::new();
    for p in ["/nd", "/nd/a", "/nd/a/b"] {
        c.post(format!("{base}/api/doc"))
            .json(&json!({ "path": p, "description": null }))
            .send()
            .await
            .unwrap();
    }
    c.put(format!("{base}/api/doc/nd/a/b/f"))
        .json(&json!({ "content": "z" }))
        .send()
        .await
        .unwrap();
    let r = c
        .delete(format!("{base}/api/doc/nd/a/b"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::CONFLICT);
    let r = c
        .delete(format!("{base}/api/doc/nd/a/b?recursive=true"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn rpc_deep_create_and_recursive_delete() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let c = reqwest::Client::new();
    for p in ["/rpc", "/rpc/deep", "/rpc/deep/dir"] {
        let r = c
            .post(format!("{base}/rpc"))
            .json(&json!({
                "jsonrpc": "2.0",
                "method": "create_directory",
                "params": { "path": p },
                "id": 1_i64,
            }))
            .send()
            .await
            .unwrap();
        let v: Value = r.json().await.unwrap();
        assert!(v.get("error").is_none(), "{p}: {v:?}");
    }
    let r = c
        .post(format!("{base}/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "create_document",
            "params": { "path": "/rpc/deep/dir/doc", "content": "rpc-body" },
            "id": 2_i64,
        }))
        .send()
        .await
        .unwrap();
    let v: Value = r.json().await.unwrap();
    assert!(v.get("error").is_none(), "{v:?}");
    let r = c
        .post(format!("{base}/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "get_document",
            "params": { "path": "/rpc/deep/dir/doc" },
            "id": 3_i64,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.json::<Value>().await.unwrap()["result"]["content"],
        "rpc-body"
    );

    let r = c
        .post(format!("{base}/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "delete_directory",
            "params": { "path": "/rpc/deep/dir", "recursive": false },
            "id": 4_i64,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.json::<Value>().await.unwrap()["error"]["code"], -32602);

    let r = c
        .post(format!("{base}/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "delete_directory",
            "params": { "path": "/rpc/deep/dir", "recursive": true },
            "id": 5_i64,
        }))
        .send()
        .await
        .unwrap();
    assert!(r.json::<Value>().await.unwrap().get("error").is_none());
    let r = c
        .post(format!("{base}/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "get_document",
            "params": { "path": "/rpc/deep/dir/doc" },
            "id": 6_i64,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.json::<Value>().await.unwrap()["error"]["code"], -32603);
}

#[tokio::test]
async fn rest_search_respects_dir_query_and_subtree() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let c = reqwest::Client::new();
    for p in ["/scope", "/scope/a", "/scope/b"] {
        c.post(format!("{base}/api/doc"))
            .json(&json!({ "path": p, "description": null }))
            .send()
            .await
            .unwrap();
    }
    c.put(format!("{base}/api/doc/scope/a/doc1"))
        .json(&json!({ "content": "keyword_a_scope_unique" }))
        .send()
        .await
        .unwrap();
    c.put(format!("{base}/api/doc/scope/b/doc2"))
        .json(&json!({ "content": "keyword_b_scope_unique" }))
        .send()
        .await
        .unwrap();

    let r = c
        .get(format!("{base}/api/search"))
        .query(&[("q", "keyword_a_scope_unique"), ("dir", "/scope/a")])
        .send()
        .await
        .unwrap();
    let hits: Vec<Value> = r.json().await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["path"], "/scope/a/doc1");

    let r = c
        .get(format!("{base}/api/search"))
        .query(&[("q", "keyword_b_scope_unique"), ("dir", "/scope")])
        .send()
        .await
        .unwrap();
    let hits: Vec<Value> = r.json().await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["path"], "/scope/b/doc2");
}

#[tokio::test]
async fn rest_put_creates_missing_deep_parents() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let c = reqwest::Client::new();
    let r = c
        .put(format!("{base}/api/doc/ghost/path/doc"))
        .json(&json!({ "content": "mkdir_p" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);
    let r = c
        .get(format!("{base}/api/doc/ghost/path/doc"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn rest_path_validation_bad_requests() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let c = reqwest::Client::new();
    for path in ["bad//slash", "trail/", "a/42/doc"] {
        let r = c
            .put(format!("{base}/api/doc/{path}"))
            .json(&json!({ "content": "x" }))
            .send()
            .await
            .unwrap();
        assert_eq!(
            r.status(),
            reqwest::StatusCode::BAD_REQUEST,
            "path={path:?}"
        );
    }
    let r = c
        .put(format!("{base}/api/doc/x%5Cy"))
        .json(&json!({ "content": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST);
}

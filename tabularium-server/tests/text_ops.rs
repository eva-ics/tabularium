//! head / tail / slice / grep / wc / stat via JSON-RPC.

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
#[allow(clippy::too_many_lines)]
async fn rpc_text_ops_match_known_content() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();

    let cat = "text_do_cat";
    let doc = "text_do_doc";
    let path = format!("/{cat}/{doc}");
    let content = "alpha\nbeta line\ngamma";

    client
        .post(format!("{base}/api/doc"))
        .json(&json!({ "path": format!("/{cat}"), "description": null }))
        .send()
        .await
        .unwrap();
    client
        .put(format!("{base}/api/doc/{cat}/{doc}"))
        .json(&json!({ "content": content }))
        .send()
        .await
        .unwrap();

    let v = rpc(
        &client,
        base,
        "head",
        json!({ "path": path, "lines": 2_u32 }),
    )
    .await;
    assert_eq!(v["result"]["text"], "alpha\nbeta line");

    let v = rpc(
        &client,
        base,
        "tail",
        json!({ "path": path, "lines": 2_u32 }),
    )
    .await;
    assert_eq!(v["result"]["text"], "beta line\ngamma");

    let v = rpc(
        &client,
        base,
        "head",
        json!({ "path": path, "lines": 0_u32 }),
    )
    .await;
    assert_eq!(v["result"]["text"], "");

    let v = rpc(
        &client,
        base,
        "tail",
        json!({ "path": path, "lines": 0_u32 }),
    )
    .await;
    assert_eq!(v["result"]["text"], "");

    let v = rpc(
        &client,
        base,
        "slice",
        json!({ "path": path, "start_line": 1_u32, "end_line": 2_u32 }),
    )
    .await;
    assert_eq!(v["result"]["text"], "alpha\nbeta line");

    let v = rpc(
        &client,
        base,
        "grep",
        json!({ "path": path, "pattern": "beta", "max_matches": 10_u64 }),
    )
    .await;
    let arr = v["result"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["line"], 2);
    assert_eq!(arr[0]["text"], "beta line");

    let v = rpc(
        &client,
        base,
        "grep",
        json!({
            "path": path,
            "pattern": "beta",
            "max_matches": 10_u64,
            "invert_match": true,
        }),
    )
    .await;
    let arr = v["result"].as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["line"], 1);
    assert_eq!(arr[0]["text"], "alpha");
    assert_eq!(arr[1]["line"], 3);
    assert_eq!(arr[1]["text"], "gamma");

    let v = rpc(&client, base, "wc", json!({ "path": path })).await;
    assert_eq!(v["result"]["bytes"], content.len() as u64);
    assert_eq!(v["result"]["lines"], 3);
    assert_eq!(v["result"]["words"], 4);
    assert_eq!(v["result"]["chars"], content.chars().count());

    let v = rpc(&client, base, "stat", json!({ "path": path })).await;
    assert_eq!(v["result"]["path"], path);
    assert_eq!(
        v["result"]["size_bytes"].as_i64().unwrap(),
        i64::try_from(content.len()).unwrap()
    );
    assert_eq!(v["result"]["line_count"], 3);
}

#[tokio::test]
async fn rpc_tail_plus_n_form() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();

    let cat = "text_tail_plus_cat";
    let doc = "text_tail_plus_doc";
    let path = format!("/{cat}/{doc}");
    let content = "alpha\nbeta line\ngamma";

    client
        .post(format!("{base}/api/doc"))
        .json(&json!({ "path": format!("/{cat}"), "description": null }))
        .send()
        .await
        .unwrap();
    client
        .put(format!("{base}/api/doc/{cat}/{doc}"))
        .json(&json!({ "content": content }))
        .send()
        .await
        .unwrap();

    let v = rpc(
        &client,
        base,
        "tail",
        json!({ "path": path, "lines": "+2" }),
    )
    .await;
    assert_eq!(v["result"]["text"], "beta line\ngamma");

    let v = rpc(
        &client,
        base,
        "tail",
        json!({ "path": path, "lines": "+1" }),
    )
    .await;
    assert_eq!(v["result"]["text"], content);

    let v = rpc(
        &client,
        base,
        "tail",
        json!({ "path": path, "lines": "+4" }),
    )
    .await;
    assert_eq!(v["result"]["text"], "");

    for bad in [json!("+0"), json!("+x"), json!(""), json!("2"), json!("+")] {
        let v = rpc(&client, base, "tail", json!({ "path": path, "lines": bad })).await;
        assert_eq!(v["error"]["code"], -32602, "bad lines: {bad}");
    }
}

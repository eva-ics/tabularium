//! WebSocket `/ws` document subscribe / append notifications.

mod common;

use common::spawn_test_server;
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn ws_subscribe_reset_then_append() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let host = base.trim_start_matches("http://");
    let ws_url = format!("ws://{host}/ws");
    let http = Client::new();
    let cat = "ws_doc_cat";
    let doc = "ws_doc_doc";
    let path = format!("/{cat}/{doc}");
    let body = "L1\nL2";

    http.post(format!("{base}/api/doc"))
        .json(&json!({ "path": format!("/{cat}"), "description": null }))
        .send()
        .await
        .unwrap();
    http.put(format!("{base}/api/doc/{cat}/{doc}"))
        .json(&json!({ "content": body }))
        .send()
        .await
        .unwrap();

    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    let sub = json!({"op":"subscribe","path": path, "lines": 10}).to_string();
    ws.send(Message::Text(sub.into())).await.unwrap();

    let raw = ws.next().await.unwrap().unwrap();
    let t = match raw {
        Message::Text(t) => t.to_string(),
        other => panic!("expected Text, got {other:?}"),
    };
    let v: Value = serde_json::from_str(&t).unwrap();
    assert_eq!(v["op"], "reset");
    assert_eq!(v["path"], path);
    assert_eq!(v["data"], body);

    http.patch(format!("{base}/api/doc/{cat}/{doc}"))
        .body("\nL3")
        .send()
        .await
        .unwrap();

    let raw2 = ws.next().await.unwrap().unwrap();
    let t2 = match raw2 {
        Message::Text(t) => t.to_string(),
        other => panic!("expected Text, got {other:?}"),
    };
    let v2: Value = serde_json::from_str(&t2).unwrap();
    assert_eq!(v2["op"], "append");
    assert_eq!(v2["path"], path);
    // Storage append inserts a newline before the piece when the doc lacks a trailing `\n`.
    assert_eq!(v2["data"], "\n\nL3");
}

#[tokio::test]
async fn ws_append_op_from_client() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let host = base.trim_start_matches("http://");
    let ws_url = format!("ws://{host}/ws");
    let http = Client::new();
    let cat = "ws_app_cat";
    let doc = "ws_app_doc";
    let path = format!("/{cat}/{doc}");

    http.post(format!("{base}/api/doc"))
        .json(&json!({ "path": format!("/{cat}"), "description": null }))
        .send()
        .await
        .unwrap();
    http.put(format!("{base}/api/doc/{cat}/{doc}"))
        .json(&json!({ "content": "x" }))
        .send()
        .await
        .unwrap();

    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    let sub = json!({"op":"subscribe","path": path, "lines": 0}).to_string();
    ws.send(Message::Text(sub.into())).await.unwrap();
    let raw0 = ws.next().await.unwrap().unwrap();
    let t0 = match raw0 {
        Message::Text(t) => t.to_string(),
        other => panic!("expected Text, got {other:?}"),
    };
    let v0: Value = serde_json::from_str(&t0).unwrap();
    assert_eq!(v0["op"], "reset");
    assert_eq!(v0["data"], "");

    let app = json!({"op":"append","path": path, "data": "y"}).to_string();
    ws.send(Message::Text(app.into())).await.unwrap();

    let raw = ws.next().await.unwrap().unwrap();
    let t = match raw {
        Message::Text(t) => t.to_string(),
        other => panic!("expected Text, got {other:?}"),
    };
    let v: Value = serde_json::from_str(&t).unwrap();
    assert_eq!(v["op"], "append");
    assert_eq!(v["data"], "\ny");
}

#[tokio::test]
async fn ws_subscribe_lines_plus_n_string() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let host = base.trim_start_matches("http://");
    let ws_url = format!("ws://{host}/ws");
    let http = Client::new();
    let cat = "ws_plus_cat";
    let doc = "ws_plus_doc";
    let path = format!("/{cat}/{doc}");
    let body = "L1\nL2\nL3";

    http.post(format!("{base}/api/doc"))
        .json(&json!({ "path": format!("/{cat}"), "description": null }))
        .send()
        .await
        .unwrap();
    http.put(format!("{base}/api/doc/{cat}/{doc}"))
        .json(&json!({ "content": body }))
        .send()
        .await
        .unwrap();

    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    let sub = json!({"op":"subscribe","path": path, "lines": "+2"}).to_string();
    ws.send(Message::Text(sub.into())).await.unwrap();

    let raw = ws.next().await.unwrap().unwrap();
    let t = match raw {
        Message::Text(t) => t.to_string(),
        other => panic!("expected Text, got {other:?}"),
    };
    let v: Value = serde_json::from_str(&t).unwrap();
    assert_eq!(v["op"], "reset");
    assert_eq!(v["data"], "L2\nL3");
}

#[tokio::test]
async fn ws_subscribe_rejects_plus_zero() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let host = base.trim_start_matches("http://");
    let ws_url = format!("ws://{host}/ws");
    let http = Client::new();
    let cat = "ws_bad_cat";
    let doc = "ws_bad_doc";
    let path = format!("/{cat}/{doc}");

    http.post(format!("{base}/api/doc"))
        .json(&json!({ "path": format!("/{cat}"), "description": null }))
        .send()
        .await
        .unwrap();
    http.put(format!("{base}/api/doc/{cat}/{doc}"))
        .json(&json!({ "content": "x" }))
        .send()
        .await
        .unwrap();

    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    let sub = json!({"op":"subscribe","path": path, "lines": "+0"}).to_string();
    ws.send(Message::Text(sub.into())).await.unwrap();

    let raw = ws.next().await.unwrap().unwrap();
    let t = match raw {
        Message::Text(t) => t.to_string(),
        other => panic!("expected Text, got {other:?}"),
    };
    let v: Value = serde_json::from_str(&t).unwrap();
    assert_eq!(v["op"], "error");
    assert!(v["message"].as_str().unwrap().contains('0'));
}

#[tokio::test]
async fn ws_say_op_appends_formatted_line() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let host = base.trim_start_matches("http://");
    let ws_url = format!("ws://{host}/ws");
    let http = Client::new();
    let cat = "ws_say_cat";
    let doc = "ws_say_doc";
    let path = format!("/{cat}/{doc}");

    http.post(format!("{base}/api/doc"))
        .json(&json!({ "path": format!("/{cat}"), "description": null }))
        .send()
        .await
        .unwrap();
    http.put(format!("{base}/api/doc/{cat}/{doc}"))
        .json(&json!({ "content": "seed\n" }))
        .send()
        .await
        .unwrap();

    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    let sub = json!({"op":"subscribe","path": path, "lines": 0}).to_string();
    ws.send(Message::Text(sub.into())).await.unwrap();
    let raw0 = ws.next().await.unwrap().unwrap();
    let t0 = match raw0 {
        Message::Text(t) => t.to_string(),
        other => panic!("expected Text, got {other:?}"),
    };
    let v0: Value = serde_json::from_str(&t0).unwrap();
    assert_eq!(v0["op"], "reset");
    assert_eq!(v0["data"], "");

    let say = json!({"op":"say","path": path, "from_id": "bob", "data": "hi"}).to_string();
    ws.send(Message::Text(say.into())).await.unwrap();

    let raw = ws.next().await.unwrap().unwrap();
    let t = match raw {
        Message::Text(t) => t.to_string(),
        other => panic!("expected Text, got {other:?}"),
    };
    let v: Value = serde_json::from_str(&t).unwrap();
    assert_eq!(v["op"], "append");
    assert_eq!(v["data"], "\n## bob\n\nhi\n\n");
}

#[tokio::test]
async fn ws_subscribe_normalizes_unrooted_and_dot_segments() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let host = base.trim_start_matches("http://");
    let ws_url = format!("ws://{host}/ws");
    let http = Client::new();
    let cat = "ws_norm_cat";
    let sub = "sub";
    let doc = "ws_norm_doc";
    let canonical = format!("/{cat}/{sub}/{doc}");
    let wire_path = format!("{cat}//{sub}/../{sub}/./{doc}");

    http.post(format!("{base}/api/doc"))
        .json(&json!({ "path": format!("/{cat}"), "description": null }))
        .send()
        .await
        .unwrap();
    http.post(format!("{base}/api/doc/{cat}"))
        .json(&json!({ "path": format!("/{cat}/{sub}"), "description": null }))
        .send()
        .await
        .unwrap();
    http.put(format!("{base}/api/doc/{cat}/{sub}/{doc}"))
        .json(&json!({ "content": "x" }))
        .send()
        .await
        .unwrap();

    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    let sub = json!({"op":"subscribe","path": wire_path, "lines": 10}).to_string();
    ws.send(Message::Text(sub.into())).await.unwrap();

    let raw = ws.next().await.unwrap().unwrap();
    let t = match raw {
        Message::Text(t) => t.to_string(),
        other => panic!("expected Text, got {other:?}"),
    };
    let v: Value = serde_json::from_str(&t).unwrap();
    assert_eq!(v["op"], "reset");
    assert_eq!(v["path"], canonical);
    assert_eq!(v["data"], "x");
}

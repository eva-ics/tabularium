//! Search hits after indexed content exists.

mod common;

use common::spawn_test_server;
use serde_json::{Value, json};

#[tokio::test]
async fn search_finds_inserted_keyword() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();

    let cat = "search_cat";
    let doc = "search_doc";
    let needle = "uniquekwtabxyz";

    client
        .post(format!("{base}/api/doc"))
        .json(&json!({ "path": format!("/{cat}"), "description": null }))
        .send()
        .await
        .unwrap();
    client
        .put(format!("{base}/api/doc/{cat}/{doc}"))
        .json(&json!({
            "content": format!("intro\n{needle} trail\n"),
        }))
        .send()
        .await
        .unwrap();

    let r = client
        .get(format!("{base}/api/search?q={needle}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let v: Value = r.json().await.unwrap();
    let hits = v.as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["path"], format!("/{cat}/{doc}"));
    let snip = hits[0]["snippet"].as_str().unwrap().to_lowercase();
    assert!(snip.contains(&needle.to_lowercase()));
    assert_eq!(hits[0]["line_number"], json!(2));
}

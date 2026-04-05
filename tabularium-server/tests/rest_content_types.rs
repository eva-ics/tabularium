//! POST / PUT / PATCH with multipart, url-encoded, and raw bodies.

mod common;

use common::spawn_test_server;
use reqwest::header::{CONTENT_TYPE, HeaderValue};
use serde_json::json;

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn rest_alternate_content_types() {
    let s = spawn_test_server().await;
    let base = &s.base_url;
    let client = reqwest::Client::new();
    let cat = "ctype_cat";

    client
        .post(format!("{base}/api/doc"))
        .json(&json!({ "path": format!("/{cat}"), "description": null }))
        .send()
        .await
        .unwrap();

    let form = reqwest::multipart::Form::new().text("content", "multipart body");
    let r = client
        .put(format!("{base}/api/doc/{cat}/multipart_doc"))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);

    let r = client
        .put(format!("{base}/api/doc/{cat}/urlenc_doc"))
        .header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        )
        .body("content=urlenc+body")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);

    let r = client
        .put(format!("{base}/api/doc/{cat}/multipart_doc"))
        .header(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )
        .body("raw put utf8")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);

    let r = client
        .get(format!("{base}/api/doc/{cat}/multipart_doc"))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["content"], "raw put utf8");

    let form = reqwest::multipart::Form::new().text("content", "multipart put");
    let r = client
        .put(format!("{base}/api/doc/{cat}/urlenc_doc"))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);

    let r = client
        .put(format!("{base}/api/doc/{cat}/urlenc_doc"))
        .header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        )
        .body("content=url+put")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);

    let r = client
        .patch(format!("{base}/api/doc/{cat}/urlenc_doc"))
        .header(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        )
        .body("content=patch+url")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);

    let r = client
        .get(format!("{base}/api/doc/{cat}/urlenc_doc"))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["content"], "url put\npatch url");

    let r = client
        .put(format!("{base}/api/doc/{cat}/multipart_doc"))
        .header(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )
        .body("\n\n")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);

    let r = client
        .get(format!("{base}/api/doc/{cat}/multipart_doc"))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["content"], "");

    let r = client
        .put(format!("{base}/api/doc/{cat}/multipart_doc"))
        .header(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )
        .body("")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);

    let r = client
        .get(format!("{base}/api/doc/{cat}/multipart_doc"))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["content"], "");

    let form = reqwest::multipart::Form::new().text("content", "patch multi");
    let r = client
        .patch(format!("{base}/api/doc/{cat}/multipart_doc"))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);

    let r = client
        .patch(format!("{base}/api/doc/{cat}/multipart_doc"))
        .header(CONTENT_TYPE, HeaderValue::from_static("text/plain"))
        .body("raw patch")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NO_CONTENT);
}

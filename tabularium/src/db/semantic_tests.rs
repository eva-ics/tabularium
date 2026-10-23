//! Contract tests using an actual HTTP mock and persisted source/sidecar databases.
use super::*;
use crate::{AuthContext, SearchKind, parse_acl_json};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

async fn embeddings(
    State(offline): State<Arc<AtomicBool>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if headers
        .get("x-embedding-test")
        .and_then(|h| h.to_str().ok())
        != Some("test-value")
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if offline.load(Ordering::SeqCst) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let rows: Vec<Value> = body["input"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let s = text.as_str().unwrap();
            let v = if s.contains("backup") || s.contains("recover") {
                vec![1.0, 0.0, 0.0]
            } else if s.contains("cake") {
                vec![0.0, 1.0, 0.0]
            } else {
                vec![0.0, 0.0, 1.0]
            };
            json!({"index":index,"embedding":v})
        })
        .rev()
        .collect(); // Client must honor indices, not response order.
    Json(json!({"data":rows})).into_response()
}
async fn mock() -> (String, Arc<AtomicBool>, tokio::task::JoinHandle<()>) {
    let offline = Arc::new(AtomicBool::new(false));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/v1/embeddings", post(embeddings))
        .with_state(offline.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, offline, task)
}
fn config(url: &str) -> SemanticConfig {
    serde_json::from_value(json!({"base_url":url,"model":"test-embedding","headers":{"x-embedding-test":"test-value"},"chunk_bytes":128,"overlap_bytes":16})).unwrap()
}
async fn open(dir: &std::path::Path, url: &str) -> SqliteDatabase {
    SqliteDatabase::init(
        format!("sqlite://{}", dir.join("source.db").display()),
        dir.join("lexical"),
        0,
    )
    .await
    .unwrap()
    .with_semantic(config(url), &dir.join("semantic"))
    .await
    .unwrap()
}

#[tokio::test]
async fn hybrid_headers_quotes_acl_and_failed_api() {
    let temp = tempfile::tempdir().unwrap();
    let (url, offline, task) = mock().await;
    let db = open(temp.path(), &url).await;
    db.create_directory("/public", None, false).await.unwrap();
    db.create_directory("/private", None, false).await.unwrap();
    let id = db
        .create_file_in_directory("/public", "recovery", "Title\nUse the backup to restart.\n")
        .await
        .unwrap();
    db.create_file_in_directory("/private", "secret", "private backup instruction")
        .await
        .unwrap();
    db.create_file_in_directory("/public", "food", "apple cake recipe")
        .await
        .unwrap();
    let before = db
        .search_report("recover", None, 10, None, SearchKind::Hybrid, None)
        .await
        .unwrap();
    assert_eq!(before.semantic_status(), "unindexed");
    assert!(
        db.search_report("recover", None, 10, None, SearchKind::Semantic, None)
            .await
            .is_err()
    );
    db.reindex_kind(None, SearchKind::Hybrid).await.unwrap();
    while db.flush_semantic_pending().await.unwrap() > 0 {}
    let acl = AuthContext::new(
        "public".into(),
        parse_acl_json(r#"{"admin":false,"allow":{"read":["/public/*"],"write":[]},"deny":{"read":["/private/*"],"write":[]}}"#).unwrap(),
    );
    let r = db
        .search_report("recover", None, 1, None, SearchKind::Hybrid, Some(&acl))
        .await
        .unwrap();
    assert_eq!(r.hits().len(), 1);
    let h = &r.hits()[0];
    assert_eq!(h.document_id(), id);
    assert_eq!(h.path(), "/public/recovery");
    assert_eq!(h.snippet(), "Title\nUse the backup to restart.\n");
    assert_eq!(h.line_number(), Some(1));
    assert_eq!(h.line_end(), Some(2));
    assert!(h.revision().is_some());
    offline.store(true, Ordering::SeqCst);
    let r = db
        .search_report(
            "backup",
            Some("/public"),
            10,
            None,
            SearchKind::Hybrid,
            None,
        )
        .await
        .unwrap();
    assert_eq!(r.semantic_status(), "unavailable");
    assert_eq!(r.hits()[0].document_id(), id);
    assert!(
        db.search_report("recover", None, 10, None, SearchKind::Semantic, None)
            .await
            .is_err()
    );
    task.abort();
}

#[tokio::test]
async fn stale_revision_retry_delete_move_and_persisted_queue() {
    let temp = tempfile::tempdir().unwrap();
    let (url, offline, task) = mock().await;
    let db = open(temp.path(), &url).await;
    db.create_directory("/old", None, false).await.unwrap();
    db.create_directory("/new", None, false).await.unwrap();
    let id = db
        .create_file_in_directory("/old", "doc", "backup version one")
        .await
        .unwrap();
    db.reindex_kind(None, SearchKind::Semantic).await.unwrap();
    db.flush_semantic_pending().await.unwrap();
    offline.store(true, Ordering::SeqCst);
    db.update_document(id, "cake version two").await.unwrap();
    assert!(db.flush_semantic_pending().await.is_err());
    offline.store(false, Ordering::SeqCst);
    let r = db
        .search_report("recover", None, 10, None, SearchKind::Semantic, None)
        .await
        .unwrap();
    assert!(
        r.hits().is_empty(),
        "stale revision must not produce a quote"
    );
    drop(db);
    let db = open(temp.path(), &url).await;
    assert!(db.flush_semantic_pending().await.unwrap() > 0);
    let r = db
        .search_report("cake", None, 10, None, SearchKind::Semantic, None)
        .await
        .unwrap();
    assert_eq!(r.hits()[0].snippet(), "cake version two");
    db.move_document_to_directory(id, "/new", "doc")
        .await
        .unwrap();
    db.flush_semantic_pending().await.unwrap();
    assert!(
        db.search_report("cake", Some("/old"), 10, None, SearchKind::Semantic, None)
            .await
            .unwrap()
            .hits()
            .is_empty()
    );
    assert_eq!(
        db.search_report("cake", Some("/new"), 10, None, SearchKind::Semantic, None)
            .await
            .unwrap()
            .hits()[0]
            .path(),
        "/new/doc"
    );
    db.delete_document(id).await.unwrap();
    assert!(
        db.search_report("cake", None, 10, None, SearchKind::Semantic, None)
            .await
            .unwrap()
            .hits()
            .is_empty()
    );
    db.flush_semantic_pending().await.unwrap();
    task.abort();
}

#[tokio::test]
async fn model_fingerprint_requires_explicit_full_rebuild() {
    let temp = tempfile::tempdir().unwrap();
    let (url, _, task) = mock().await;
    let db = open(temp.path(), &url).await;
    db.create_file_in_directory("/", "doc", "backup")
        .await
        .unwrap();
    db.reindex_kind(None, SearchKind::Semantic).await.unwrap();
    drop(db);
    let mut cfg = config(&url);
    cfg.model_revision = "different".into();
    let db = SqliteDatabase::init(
        format!("sqlite://{}", temp.path().join("source.db").display()),
        temp.path().join("lexical"),
        0,
    )
    .await
    .unwrap()
    .with_semantic(cfg, &temp.path().join("semantic"))
    .await
    .unwrap();
    assert_eq!(
        db.search_report("recover", None, 10, None, SearchKind::Hybrid, None)
            .await
            .unwrap()
            .semantic_status(),
        "incompatible"
    );
    assert!(
        db.reindex_kind(Some("/x"), SearchKind::Semantic)
            .await
            .is_err()
    );
    db.reindex_kind(None, SearchKind::Semantic).await.unwrap();
    assert!(
        !db.search_report("recover", None, 10, None, SearchKind::Semantic, None)
            .await
            .unwrap()
            .hits()
            .is_empty()
    );
    task.abort();
}

#[tokio::test]
async fn failed_rebuild_preserves_active_index_and_scoped_reindex_isolated() {
    let temp = tempfile::tempdir().unwrap();
    let (url, offline, task) = mock().await;
    let db = open(temp.path(), &url).await;
    for path in ["/a", "/ab"] {
        db.create_directory(path, None, false).await.unwrap();
    }
    db.create_file_in_directory("/a", "doc", "backup alpha")
        .await
        .unwrap();
    db.create_file_in_directory("/ab", "doc", "cake beta")
        .await
        .unwrap();
    db.reindex_kind(None, SearchKind::Semantic).await.unwrap();
    while db.flush_semantic_pending().await.unwrap() > 0 {}
    offline.store(true, Ordering::SeqCst);
    assert!(db.reindex_kind(None, SearchKind::Semantic).await.is_err());
    offline.store(false, Ordering::SeqCst);
    let r = db
        .search_report("recover", Some("/a"), 10, None, SearchKind::Semantic, None)
        .await
        .unwrap();
    assert_eq!(r.hits().len(), 1);
    assert_eq!(r.hits()[0].path(), "/a/doc");
    db.reindex_kind(Some("/a"), SearchKind::Semantic)
        .await
        .unwrap();
    let r = db
        .search_report("cake", Some("/ab"), 10, None, SearchKind::Semantic, None)
        .await
        .unwrap();
    assert_eq!(r.hits()[0].path(), "/ab/doc");
    let index = db.semantic.as_ref().unwrap();
    index.queue(999).await.unwrap();
    let generation = index
        .pending(16)
        .await
        .unwrap()
        .into_iter()
        .find(|(id, _)| *id == 999)
        .unwrap()
        .1;
    index.queue(999).await.unwrap();
    index.acknowledge(999, generation).await.unwrap();
    assert!(
        index
            .pending(16)
            .await
            .unwrap()
            .iter()
            .any(|(id, _)| *id == 999)
    );
    task.abort();
}

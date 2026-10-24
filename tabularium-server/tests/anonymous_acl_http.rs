//! `[anonymous]` path ACL — credentials absent use the configured principal; invalid keys stay 401.

use std::sync::Arc;
use std::time::Duration;

use bma_ts::Monotonic;
use serde_json::json;
use tabularium::jsonrpc_codes::{FORBIDDEN, UNAUTHORIZED};
use tabularium::{AclBodyJson, AclSideJson, AuthContext, SqliteDatabase};
use tabularium_server::web::{AppState, router};
use tokio::net::TcpListener;

async fn spawn_anonymous_server() -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("anon.db");
    let idx_path = dir.path().join("anon.idx");
    let uri = format!("sqlite://{}", db_path.display());
    let db = Arc::new(SqliteDatabase::init(&uri, &idx_path, 8).await.unwrap());
    db.create_directory("/wiki", None, false).await.unwrap();
    db.create_directory("/sandbox", None, false).await.unwrap();
    db.create_directory("/private", None, false).await.unwrap();
    db.create_document_at_path("/wiki/home.md", "# wiki\n", false, None)
        .await
        .unwrap();
    db.create_document_at_path("/sandbox/note.md", "# sandbox\n", false, None)
        .await
        .unwrap();
    db.create_document_at_path("/private/secret.md", "# secret\n", false, None)
        .await
        .unwrap();

    let admin_acl =
        r#"{"admin":true,"allow":{"read":[],"write":[]},"deny":{"read":[],"write":[]}}"#;
    db.acl_upsert_validated("admin", admin_acl).await.unwrap();
    db.psk_insert("admin-local", "admin", "admin-psk-abcdefghijklmnopqrstuv")
        .await
        .unwrap();

    let anonymous = AuthContext::new(
        "anonymous".into(),
        AclBodyJson {
            admin: false,
            allow: AclSideJson {
                read: vec![
                    "/wiki".into(),
                    "/wiki/*".into(),
                    "/sandbox".into(),
                    "/sandbox/*".into(),
                ],
                write: vec!["/sandbox".into(), "/sandbox/*".into()],
            },
            deny: AclSideJson {
                read: vec![],
                write: vec![],
            },
        },
    );

    let app = router(AppState {
        db,
        wait_timeout: Duration::from_secs(3),
        process_started_at: Monotonic::now(),
        authenticate_api: true,
        authenticate_mcp: false,
        oidc: None,
        anonymous: Some(anonymous),
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(120)).await;
    (format!("http://{}", addr), dir)
}

#[tokio::test]
async fn health_stays_open_with_anonymous() {
    let (base, _dir) = spawn_anonymous_server().await;
    let c = reqwest::Client::new();
    let r = c.get(format!("{base}/api/test")).send().await.unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn anonymous_whoami_and_wiki_read() {
    let (base, _dir) = spawn_anonymous_server().await;
    let c = reqwest::Client::new();
    let w = c.get(format!("{base}/api/whoami")).send().await.unwrap();
    assert_eq!(w.status(), reqwest::StatusCode::OK);
    let who: serde_json::Value = w.json().await.unwrap();
    assert_eq!(who["acl"], "anonymous");
    assert_eq!(who["anonymous"], true);
    assert_eq!(who["admin"], false);

    let r = c
        .get(format!("{base}/api/doc/wiki/home.md"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);

    let list = c.get(format!("{base}/api/doc")).send().await.unwrap();
    assert_eq!(list.status(), reqwest::StatusCode::OK);
    let rows: Vec<serde_json::Value> = list.json().await.unwrap();
    let names: Vec<&str> = rows.iter().filter_map(|r| r["name"].as_str()).collect();
    assert!(names.contains(&"wiki"));
    assert!(names.contains(&"sandbox"));
    assert!(!names.contains(&"private"));
}

#[tokio::test]
async fn anonymous_cannot_write_wiki_or_read_private() {
    let (base, _dir) = spawn_anonymous_server().await;
    let c = reqwest::Client::new();
    let put = c
        .put(format!("{base}/api/doc/wiki/home.md"))
        .header("content-type", "application/json")
        .json(&json!({"content": "# hacked\n"}))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), reqwest::StatusCode::FORBIDDEN);

    let secret = c
        .get(format!("{base}/api/doc/private/secret.md"))
        .send()
        .await
        .unwrap();
    assert_eq!(secret.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn anonymous_can_write_sandbox() {
    let (base, _dir) = spawn_anonymous_server().await;
    let c = reqwest::Client::new();
    let put = c
        .put(format!("{base}/api/doc/sandbox/note.md"))
        .header("content-type", "application/json")
        .json(&json!({"content": "# updated\n"}))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), reqwest::StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn unknown_psk_is_unauthorized_not_anonymous() {
    let (base, _dir) = spawn_anonymous_server().await;
    let c = reqwest::Client::new();
    let r = c
        .get(format!("{base}/api/whoami"))
        .header("X-Auth-Key", "not-a-real-key")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::UNAUTHORIZED);

    let empty = c
        .get(format!("{base}/api/whoami"))
        .header("X-Auth-Key", "   ")
        .send()
        .await
        .unwrap();
    assert_eq!(empty.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn anonymous_rpc_admin_forbidden_admin_psk_ok() {
    let (base, _dir) = spawn_anonymous_server().await;
    let c = reqwest::Client::new();
    let body = json!({
        "jsonrpc": "2.0",
        "method": "acl_list",
        "params": {},
        "id": 1_i64,
    });
    let denied = c
        .post(format!("{base}/rpc"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: serde_json::Value = denied.json().await.unwrap();
    assert_eq!(v["error"]["code"], FORBIDDEN as i64);

    let ok = c
        .post(format!("{base}/rpc"))
        .header("X-Auth-Key", "admin-psk-abcdefghijklmnopqrstuv")
        .json(&body)
        .send()
        .await
        .unwrap();
    let v: serde_json::Value = ok.json().await.unwrap();
    assert!(v.get("error").is_none(), "{v:?}");
    assert!(v["result"].is_array());

    let miss = c
        .post(format!("{base}/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "method": "get_document",
            "params": {"path": "/private/secret.md"},
            "id": 2_i64,
        }))
        .send()
        .await
        .unwrap();
    let v: serde_json::Value = miss.json().await.unwrap();
    // Forbidden for anonymous path deny (may surface as -32004).
    assert!(
        v["error"]["code"] == FORBIDDEN as i64 || v["error"]["code"] == UNAUTHORIZED as i64,
        "{v:?}"
    );
}

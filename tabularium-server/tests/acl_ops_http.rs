//! ACL matrix — read RPCs + REST listing vs narrow ACL shapes (catches regressions across surfaces).

use std::sync::Arc;
use std::time::Duration;

use bma_ts::Monotonic;
use serde_json::{Value, json};
use tabularium::SqliteDatabase;
use tabularium::jsonrpc_codes::FORBIDDEN;
use tabularium_server::web::{AppState, router};
use tokio::net::TcpListener;

struct AclOpsFixture {
    base: String,
    _dir: tempfile::TempDir,
    key_subtree: String,
    key_exact: String,
    key_nomatch: String,
    key_denyov: String,
}

async fn rpc_json(
    client: &reqwest::Client,
    base: &str,
    key: &str,
    method: &str,
    params: Value,
) -> Value {
    let body = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1_u64,
    });
    let r = client
        .post(format!("{base}/rpc"))
        .header("X-Auth-Key", key)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    r.json().await.unwrap()
}

fn assert_rpc_ok(v: &Value, ctx: &str) {
    assert!(v.get("error").is_none(), "{ctx}: {v:?}");
}

fn assert_rpc_forbidden(v: &Value, ctx: &str) {
    assert_eq!(
        v["error"]["code"].as_i64(),
        Some(i64::from(FORBIDDEN)),
        "{ctx}: {v:?}"
    );
}

fn read_file_rpc_matrix(path: &str, grep_pat: &str) -> Vec<(&'static str, Value)> {
    vec![
        ("get_document", json!({ "path": path })),
        (
            "grep",
            json!({
                "path": path,
                "pattern": grep_pat,
                "max_matches": 20_u32
            }),
        ),
        ("wc", json!({ "path": path })),
        ("stat", json!({ "path": path })),
        (
            "slice",
            json!({ "path": path, "start_line": 1_u32, "end_line": 50_u32 }),
        ),
        ("head", json!({ "path": path, "lines": 50_u32 })),
        ("tail", json!({ "path": path, "lines": 50_u32 })),
    ]
}

async fn spawn_acl_ops_fixture() -> AclOpsFixture {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("acl_ops.db");
    let idx_path = dir.path().join("acl_ops.idx");
    let uri = format!("sqlite://{}", db_path.display());
    let db = Arc::new(SqliteDatabase::init(&uri, &idx_path, 8).await.unwrap());

    db.create_directory("/test", None, false).await.unwrap();
    db.create_directory("/test/nested", None, false)
        .await
        .unwrap();
    db.create_directory("/other", None, false).await.unwrap();

    db.create_document_at_path("/test/file.txt", "hello world\nline2\n", false, None)
        .await
        .unwrap();
    db.create_document_at_path("/test/nested/doc.txt", "nested content\n", false, None)
        .await
        .unwrap();
    db.create_document_at_path("/test/secret.txt", "classified\n", false, None)
        .await
        .unwrap();
    db.create_document_at_path("/other/file.txt", "other content\n", false, None)
        .await
        .unwrap();

    let subtree_acl =
        r#"{"admin":false,"allow":{"read":["/test/*"],"write":[]},"deny":{"read":[],"write":[]}}"#;
    let exact_acl = r#"{"admin":false,"allow":{"read":["/test/file.txt"],"write":[]},"deny":{"read":[],"write":[]}}"#;
    let nomatch_acl =
        r#"{"admin":false,"allow":{"read":["/other/*"],"write":[]},"deny":{"read":[],"write":[]}}"#;
    let deny_ov_acl = r#"{"admin":false,"allow":{"read":["/test/*"],"write":[]},"deny":{"read":["/test/secret.txt"],"write":[]}}"#;

    db.acl_upsert_validated("subtree", subtree_acl)
        .await
        .unwrap();
    db.acl_upsert_validated("exact", exact_acl).await.unwrap();
    db.acl_upsert_validated("nomatch", nomatch_acl)
        .await
        .unwrap();
    db.acl_upsert_validated("deny-ov", deny_ov_acl)
        .await
        .unwrap();

    let key_subtree = "subtree-aclops-psk-abcdefghijklmnopqrst".to_string();
    let key_exact = "exact-aclops-psk-abcdefghijklmnopqrstuv".to_string();
    let key_nomatch = "nomatch-aclops-psk-abcdefghijklmnopqrs".to_string();
    let key_denyov = "denyov-aclops-psk-abcdefghijklmnopqrstuv".to_string();

    db.psk_insert("k-subtree", "subtree", &key_subtree)
        .await
        .unwrap();
    db.psk_insert("k-exact", "exact", &key_exact).await.unwrap();
    db.psk_insert("k-nomatch", "nomatch", &key_nomatch)
        .await
        .unwrap();
    db.psk_insert("k-denyov", "deny-ov", &key_denyov)
        .await
        .unwrap();

    let app = router(AppState {
        db: Arc::clone(&db),
        wait_timeout: Duration::from_secs(3),
        process_started_at: Monotonic::now(),
        authenticate_api: true,
        authenticate_mcp: false,
        oidc: None,
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(150)).await;

    AclOpsFixture {
        base: format!("http://{}", addr),
        _dir: dir,
        key_subtree,
        key_exact,
        key_nomatch,
        key_denyov,
    }
}

#[tokio::test]
async fn acl_ops_subtree_psk_allows_all_read_rpcs_on_covered_files() {
    let f = spawn_acl_ops_fixture().await;
    let c = reqwest::Client::new();

    for (path, pat) in [
        ("/test/file.txt", "hello"),
        ("/test/nested/doc.txt", "nested"),
    ] {
        for (method, params) in read_file_rpc_matrix(path, pat) {
            let v = rpc_json(&c, &f.base, &f.key_subtree, method, params).await;
            assert_rpc_ok(&v, &format!("subtree {method} {path}"));
        }
    }
}

#[tokio::test]
async fn acl_ops_subtree_psk_forbids_uncovered_file_reads() {
    let f = spawn_acl_ops_fixture().await;
    let c = reqwest::Client::new();
    let path = "/other/file.txt";
    for (method, params) in read_file_rpc_matrix(path, "other") {
        let v = rpc_json(&c, &f.base, &f.key_subtree, method, params).await;
        assert_rpc_forbidden(&v, &format!("subtree {method} {path}"));
    }
}

#[tokio::test]
async fn acl_ops_exact_psk_allows_only_whitelisted_file() {
    let f = spawn_acl_ops_fixture().await;
    let c = reqwest::Client::new();
    let path = "/test/file.txt";
    for (method, params) in read_file_rpc_matrix(path, "hello") {
        let v = rpc_json(&c, &f.base, &f.key_exact, method, params).await;
        assert_rpc_ok(&v, &format!("exact {method} {path}"));
    }
}

#[tokio::test]
async fn acl_ops_exact_psk_forbids_non_whitelisted_file_under_test() {
    let f = spawn_acl_ops_fixture().await;
    let c = reqwest::Client::new();
    let path = "/test/nested/doc.txt";
    for (method, params) in read_file_rpc_matrix(path, "nested") {
        let v = rpc_json(&c, &f.base, &f.key_exact, method, params).await;
        assert_rpc_forbidden(&v, &format!("exact {method} {path}"));
    }
}

#[tokio::test]
async fn acl_ops_nomatch_psk_forbids_test_tree_files() {
    let f = spawn_acl_ops_fixture().await;
    let c = reqwest::Client::new();
    let path = "/test/file.txt";
    for (method, params) in read_file_rpc_matrix(path, "hello") {
        let v = rpc_json(&c, &f.base, &f.key_nomatch, method, params).await;
        assert_rpc_forbidden(&v, &format!("nomatch {method} {path}"));
    }
}

#[tokio::test]
async fn acl_ops_deny_override_blocks_denied_file_only() {
    let f = spawn_acl_ops_fixture().await;
    let c = reqwest::Client::new();

    let secret = "/test/secret.txt";
    for (method, params) in read_file_rpc_matrix(secret, "classified") {
        let v = rpc_json(&c, &f.base, &f.key_denyov, method, params).await;
        assert_rpc_forbidden(&v, &format!("denyov {method} {secret}"));
    }

    let ok_path = "/test/file.txt";
    for (method, params) in read_file_rpc_matrix(ok_path, "hello") {
        let v = rpc_json(&c, &f.base, &f.key_denyov, method, params).await;
        assert_rpc_ok(&v, &format!("denyov {method} {ok_path}"));
    }
}

#[tokio::test]
async fn acl_ops_deny_override_search_scoped_tree_excludes_denied_file_rpc() {
    let f = spawn_acl_ops_fixture().await;
    let c = reqwest::Client::new();

    let v = rpc_json(
        &c,
        &f.base,
        &f.key_denyov,
        "search",
        json!({ "query": "classified", "path": "/test" }),
    )
    .await;
    assert_rpc_ok(&v, "denyov rpc search classified scoped");
    assert!(
        v["result"].as_array().unwrap().is_empty(),
        "denied body must not surface as a hit: {v:?}"
    );

    let v = rpc_json(
        &c,
        &f.base,
        &f.key_denyov,
        "search",
        json!({ "query": "hello", "path": "/test" }),
    )
    .await;
    assert_rpc_ok(&v, "denyov rpc search hello scoped");
    let paths: Vec<&str> = v["result"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h["path"].as_str())
        .collect();
    assert!(paths.contains(&"/test/file.txt"));
    assert!(!paths.iter().any(|p| *p == "/test/secret.txt"));
}

#[tokio::test]
async fn acl_ops_deny_override_search_scoped_tree_excludes_denied_file_rest() {
    let f = spawn_acl_ops_fixture().await;
    let c = reqwest::Client::new();

    let r = c
        .get(format!("{}/api/search", f.base))
        .header("X-Auth-Key", &f.key_denyov)
        .query(&[("q", "classified"), ("dir", "/test")])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let body: Value = r.json().await.unwrap();
    assert!(
        body.as_array().unwrap().is_empty(),
        "REST scoped search must omit denied doc: {body:?}"
    );

    let r = c
        .get(format!("{}/api/search", f.base))
        .header("X-Auth-Key", &f.key_denyov)
        .query(&[("q", "hello"), ("dir", "/test")])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let body: Value = r.json().await.unwrap();
    let paths: Vec<&str> = body
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h["path"].as_str())
        .collect();
    assert!(paths.contains(&"/test/file.txt"));
    assert!(!paths.iter().any(|p| *p == "/test/secret.txt"));
}

#[tokio::test]
async fn acl_ops_rest_list_root_filtered_for_subtree_psk() {
    let f = spawn_acl_ops_fixture().await;
    let c = reqwest::Client::new();
    let r = c
        .get(format!("{}/api/doc", f.base))
        .header("X-Auth-Key", &f.key_subtree)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let rows = r.json::<Value>().await.unwrap();
    let names: Vec<&str> = rows
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row["name"].as_str())
        .collect();
    assert!(names.contains(&"test"));
    assert!(!names.contains(&"other"));
}

#[tokio::test]
async fn acl_ops_rest_list_test_returns_visible_children_for_subtree_psk() {
    let f = spawn_acl_ops_fixture().await;
    let c = reqwest::Client::new();
    let r = c
        .get(format!("{}/api/doc/test", f.base))
        .header("X-Auth-Key", &f.key_subtree)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let rows = r.json::<Value>().await.unwrap();
    let names: Vec<&str> = rows
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row["name"].as_str())
        .collect();
    assert!(names.contains(&"file.txt"));
    assert!(names.contains(&"nested"));
}

#[tokio::test]
async fn acl_ops_rest_list_other_empty_for_subtree_psk() {
    let f = spawn_acl_ops_fixture().await;
    let c = reqwest::Client::new();
    let r = c
        .get(format!("{}/api/doc/other", f.base))
        .header("X-Auth-Key", &f.key_subtree)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let rows = r.json::<Value>().await.unwrap();
    assert!(rows.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn acl_ops_exists_subtree_matrix() {
    let f = spawn_acl_ops_fixture().await;
    let c = reqwest::Client::new();

    let v = rpc_json(
        &c,
        &f.base,
        &f.key_subtree,
        "exists",
        json!({ "path": "/test/file.txt" }),
    )
    .await;
    assert_rpc_ok(&v, "exists file");
    assert_eq!(v["result"], true);

    for dir_path in ["/test", "/test/nested"] {
        let v = rpc_json(
            &c,
            &f.base,
            &f.key_subtree,
            "exists",
            json!({ "path": dir_path }),
        )
        .await;
        assert_rpc_ok(&v, &format!("exists dir {dir_path}"));
        assert_eq!(v["result"], false);
    }

    let v = rpc_json(
        &c,
        &f.base,
        &f.key_nomatch,
        "exists",
        json!({ "path": "/test/file.txt" }),
    )
    .await;
    assert_rpc_forbidden(&v, "exists nomatch covered file");
}

#[tokio::test]
async fn acl_ops_search_subtree_filters_hits() {
    let f = spawn_acl_ops_fixture().await;
    let c = reqwest::Client::new();

    let v = rpc_json(
        &c,
        &f.base,
        &f.key_subtree,
        "search",
        json!({ "query": "hello" }),
    )
    .await;
    assert_rpc_ok(&v, "search subtree hello");
    let paths: Vec<&str> = v["result"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h["path"].as_str())
        .collect();
    assert!(paths.iter().any(|p| *p == "/test/file.txt"));
    assert!(!paths.iter().any(|p| *p == "/other/file.txt"));

    let v = rpc_json(
        &c,
        &f.base,
        &f.key_subtree,
        "search",
        json!({ "query": "other" }),
    )
    .await;
    assert_rpc_ok(&v, "search subtree other");
    let paths: Vec<&str> = v["result"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h["path"].as_str())
        .collect();
    assert!(!paths.iter().any(|p| *p == "/other/file.txt"));

    let v = rpc_json(
        &c,
        &f.base,
        &f.key_nomatch,
        "search",
        json!({ "query": "hello" }),
    )
    .await;
    assert_rpc_ok(&v, "search nomatch");
    assert!(v["result"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn acl_ops_exact_psk_search_allows_ancestor_path_scope_rpc() {
    let f = spawn_acl_ops_fixture().await;
    let c = reqwest::Client::new();
    let v = rpc_json(
        &c,
        &f.base,
        &f.key_exact,
        "search",
        json!({ "query": "hello", "path": "/test" }),
    )
    .await;
    assert_rpc_ok(&v, "search exact ancestor");
    let paths: Vec<&str> = v["result"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h["path"].as_str())
        .collect();
    assert_eq!(paths, vec!["/test/file.txt"]);
}

#[tokio::test]
async fn acl_ops_exact_psk_search_nested_scope_is_forbidden_rpc() {
    let f = spawn_acl_ops_fixture().await;
    let c = reqwest::Client::new();
    let v = rpc_json(
        &c,
        &f.base,
        &f.key_exact,
        "search",
        json!({ "query": "nested", "path": "/test/nested" }),
    )
    .await;
    assert_rpc_forbidden(&v, "search exact nested dir no traverse");
}

#[tokio::test]
async fn acl_ops_exact_psk_search_exact_file_path_rpc() {
    let f = spawn_acl_ops_fixture().await;
    let c = reqwest::Client::new();
    let v = rpc_json(
        &c,
        &f.base,
        &f.key_exact,
        "search",
        json!({ "query": "hello", "path": "/test/file.txt" }),
    )
    .await;
    assert_rpc_ok(&v, "search exact file path");
    let paths: Vec<&str> = v["result"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h["path"].as_str())
        .collect();
    assert_eq!(paths, vec!["/test/file.txt"]);
}

#[tokio::test]
async fn acl_ops_exact_psk_search_traverse_matches_rest_get() {
    let f = spawn_acl_ops_fixture().await;
    let c = reqwest::Client::new();
    let r = c
        .get(format!("{}/api/search", f.base))
        .header("X-Auth-Key", &f.key_exact)
        .query(&[("q", "hello"), ("dir", "/test")])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let body: Value = r.json().await.unwrap();
    let paths: Vec<&str> = body
        .as_array()
        .expect("search array")
        .iter()
        .filter_map(|h| h["path"].as_str())
        .collect();
    assert_eq!(paths, vec!["/test/file.txt"]);
}

#[tokio::test]
async fn acl_ops_nomatch_psk_search_foreign_directory_forbidden() {
    let f = spawn_acl_ops_fixture().await;
    let c = reqwest::Client::new();
    let v = rpc_json(
        &c,
        &f.base,
        &f.key_nomatch,
        "search",
        json!({ "query": "hello", "path": "/test" }),
    )
    .await;
    assert_rpc_forbidden(&v, "search nomatch foreign dir");
}

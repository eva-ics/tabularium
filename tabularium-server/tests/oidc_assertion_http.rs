//! `[oidc]` signed assertion header — Omnissiah verifies the seal before the gate opens.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use bma_ts::Monotonic;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::json;
use tabularium::SqliteDatabase;
use tabularium::jsonrpc_codes::UNAUTHORIZED;
use tabularium_server::config::OidcSection;
use tabularium_server::jwt_assertion::AssertionRuntime;
use tabularium_server::web::{AppState, router};
use tempfile::TempDir;
use tokio::net::TcpListener;

fn jwt_exp_soon() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 7200
}

fn mint_hs256(secret: &[u8], groups: &[&str]) -> String {
    let mut header = Header::new(Algorithm::HS256);
    header.kid = Some("kid1".into());
    let claims = json!({
        "exp": jwt_exp_soon(),
        "groups": groups,
    });
    encode(&header, &claims, &EncodingKey::from_secret(secret)).unwrap()
}

async fn spawn_oidc_authenticated_server() -> (String, Vec<u8>, &'static str, TempDir) {
    let dir = TempDir::new().unwrap();
    let secret: Vec<u8> = b"unit-test-hs256-secret-key-bytes!".to_vec();
    let k_b64 = URL_SAFE_NO_PAD.encode(&secret);
    let jwks_path = dir.path().join("jwks.json");
    std::fs::write(
        &jwks_path,
        json!({"keys":[{
            "kty": "oct",
            "alg": "HS256",
            "kid": "kid1",
            "k": k_b64
        }]})
        .to_string(),
    )
    .unwrap();

    let db_path = dir.path().join("t.db");
    let idx_path = dir.path().join("t.idx");
    let uri = format!("sqlite://{}", db_path.display());
    let db = Arc::new(SqliteDatabase::init(&uri, &idx_path, 8).await.unwrap());
    let acl = r#"{"admin":true,"allow":{"read":[],"write":[]},"deny":{"read":[],"write":[]}}"#;
    db.acl_upsert_validated("jwt_role", acl).await.unwrap();
    db.psk_insert("op", "jwt_role", "thepsk").await.unwrap();

    let oidc_cfg = OidcSection {
        key: jwks_path.to_str().unwrap().to_string(),
        header: "X-JWT-Assertion".into(),
        groups_field: "groups".into(),
        refresh: 3600,
        retry: 1,
        timeout: 10,
        group_name_prefix: None,
    };
    let oidc = Arc::new(AssertionRuntime::bootstrap(oidc_cfg).await.unwrap());
    let app = router(AppState {
        db,
        wait_timeout: Duration::from_secs(3),
        process_started_at: Monotonic::now(),
        authenticate_api: true,
        authenticate_mcp: false,
        oidc: Some(oidc),
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(120)).await;
    (format!("http://{}", addr), secret, "thepsk", dir)
}

#[tokio::test]
async fn api_test_reports_oidc_enabled() {
    let (base, _secret, _psk, _dir) = spawn_oidc_authenticated_server().await;
    let r = reqwest::Client::new()
        .get(format!("{base}/api/test"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let v: serde_json::Value = r.json().await.unwrap();
    assert_eq!(v["authenticate_api"], true);
    assert_eq!(v["oidc_enabled"], true);
}

#[tokio::test]
async fn rpc_accepts_valid_jwt_assertion_mapped_to_acl() {
    let (base, secret, _psk, _dir) = spawn_oidc_authenticated_server().await;
    let token = mint_hs256(&secret, &["jwt_role"]);
    let c = reqwest::Client::new();
    let body = json!({
        "jsonrpc": "2.0",
        "method": "list_directory",
        "params": { "path": "/" },
        "id": 1_i64,
    });
    let r = c
        .post(format!("{base}/rpc"))
        .header("X-JWT-Assertion", token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let v: serde_json::Value = r.json().await.unwrap();
    assert!(v.get("error").is_none(), "{v:?}");
}

#[tokio::test]
async fn rpc_rejects_bad_jwt_when_oidc_configured() {
    let (base, _secret, _psk, _dir) = spawn_oidc_authenticated_server().await;
    let c = reqwest::Client::new();
    let body = json!({
        "jsonrpc": "2.0",
        "method": "list_directory",
        "params": { "path": "/" },
        "id": 1_i64,
    });
    let r = c
        .post(format!("{base}/rpc"))
        .header("X-JWT-Assertion", "not.a.jwt")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let v: serde_json::Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], i64::from(UNAUTHORIZED));
}

#[tokio::test]
async fn rpc_does_not_fallback_to_psk_when_assertion_header_present_but_invalid() {
    let (base, _secret, psk, _dir) = spawn_oidc_authenticated_server().await;
    let wrong = mint_hs256(b"different-secret-key-bytes-!!!!!", &["jwt_role"]);
    let c = reqwest::Client::new();
    let body = json!({
        "jsonrpc": "2.0",
        "method": "list_directory",
        "params": { "path": "/" },
        "id": 1_i64,
    });
    let r = c
        .post(format!("{base}/rpc"))
        .header("X-JWT-Assertion", wrong)
        .header("X-Auth-Key", psk)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let v: serde_json::Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], i64::from(UNAUTHORIZED));
}

#[tokio::test]
async fn rpc_falls_back_to_psk_when_no_assertion_header() {
    let (base, _secret, psk, _dir) = spawn_oidc_authenticated_server().await;
    let c = reqwest::Client::new();
    let body = json!({
        "jsonrpc": "2.0",
        "method": "list_directory",
        "params": { "path": "/" },
        "id": 1_i64,
    });
    let r = c
        .post(format!("{base}/rpc"))
        .header("X-Auth-Key", psk)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let v: serde_json::Value = r.json().await.unwrap();
    assert!(v.get("error").is_none(), "{v:?}");
}

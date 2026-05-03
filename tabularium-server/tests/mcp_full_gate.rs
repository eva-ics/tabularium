//! MCP `mcp.full` tool registration (safe vs trusted full surface).

#![cfg(feature = "mcp")]

use std::sync::Arc;
use std::time::Duration;

use bma_ts::Monotonic;
use tabularium::SqliteDatabase;
use tabularium_server::mcp::TabulariumMcp;
use tabularium_server::web::AppState;

async fn test_app() -> (AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("mcp_gate.db");
    let idx_path = dir.path().join("mcp_gate.idx");
    let uri = format!("sqlite://{}", db_path.display());
    let db = Arc::new(
        SqliteDatabase::init(&uri, &idx_path, 8)
            .await
            .expect("db init"),
    );
    let app = AppState {
        db,
        wait_timeout: Duration::from_secs(3),
        process_started_at: Monotonic::now(),
    };
    (app, dir)
}

#[tokio::test]
async fn mcp_full_false_omits_destructive_tools() {
    let (app, _dir) = test_app().await;
    let mcp = TabulariumMcp::new(app, Arc::from(""), false);
    assert!(!mcp.has_mcp_tool("delete_document"));
    assert!(!mcp.has_mcp_tool("reindex"));
    assert!(mcp.has_mcp_tool("get_document"));
    assert!(mcp.has_mcp_tool("list_directory"));
}

#[tokio::test]
async fn mcp_full_true_registers_destructive_tools() {
    let (app, _dir) = test_app().await;
    let mcp = TabulariumMcp::new(app, Arc::from(""), true);
    assert!(mcp.has_mcp_tool("delete_document"));
    assert!(mcp.has_mcp_tool("delete_directory"));
    assert!(mcp.has_mcp_tool("rename_document"));
    assert!(mcp.has_mcp_tool("rename_directory"));
    assert!(mcp.has_mcp_tool("move_document"));
    assert!(mcp.has_mcp_tool("move_directory"));
    assert!(mcp.has_mcp_tool("reindex"));
}

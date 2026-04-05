//! Shared ephemeral server for integration tests.

use std::sync::Arc;
use std::time::Duration;

use bma_ts::Monotonic;
use tabularium::SqliteDatabase;
use tabularium_server::web::{AppState, router};
use tokio::net::TcpListener;

/// Keeps the temp DB/index alive while the server runs.
pub struct TestServer {
    pub base_url: String,
    _dir: tempfile::TempDir,
}

pub async fn spawn_test_server() -> TestServer {
    spawn_test_server_with_wait_timeout(Duration::from_secs(3)).await
}

pub async fn spawn_test_server_with_wait_timeout(wait_timeout: Duration) -> TestServer {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("t.db");
    let idx_path = dir.path().join("t.idx");
    let uri = format!("sqlite://{}", db_path.display());
    let db = Arc::new(SqliteDatabase::init(&uri, &idx_path, 8).await.unwrap());
    let app = router(AppState {
        db,
        wait_timeout,
        process_started_at: Monotonic::now(),
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(80)).await;

    TestServer {
        base_url: format!("http://{}", addr),
        _dir: dir,
    }
}

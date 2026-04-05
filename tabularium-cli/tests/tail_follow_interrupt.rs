use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

use bma_ts::Monotonic;
use tabularium::SqliteDatabase;
use tabularium::rpc::Client;
use tabularium_server::web::{AppState, router};
use tempfile::TempDir;
use tokio::net::TcpListener;

struct TestServer {
    base_url: String,
    _dir: TempDir,
}

async fn spawn_test_server() -> TestServer {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("t.db");
    let idx_path = dir.path().join("t.idx");
    let uri = format!("sqlite://{}", db_path.display());
    let db = Arc::new(SqliteDatabase::init(&uri, &idx_path, 8).await.unwrap());
    let app = router(AppState {
        db,
        wait_timeout: Duration::from_secs(3),
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

#[cfg(unix)]
#[tokio::test]
async fn tail_follow_interrupted_by_sigint() {
    let server = spawn_test_server().await;
    let client = Client::init(&server.base_url, Duration::from_secs(5)).unwrap();
    client
        .create_directory("/tail_follow_cat", None)
        .await
        .unwrap();
    client
        .put_document("/tail_follow_cat/doc", "one\ntwo\n")
        .await
        .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_tb"))
        .arg("-u")
        .arg(&server.base_url)
        .arg("tail")
        .arg("-f")
        .arg("/tail_follow_cat/doc")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    tokio::time::sleep(Duration::from_millis(250)).await;
    let pid: i32 = child.id().try_into().expect("process id should fit pid_t");
    unsafe {
        libc::kill(pid, libc::SIGINT);
    }

    let status = tokio::task::spawn_blocking(move || child.wait())
        .await
        .unwrap()
        .unwrap();
    let by_exit_130 = status.code() == Some(130);
    let by_signal = status.signal() == Some(libc::SIGINT);
    assert!(
        by_exit_130 || by_signal,
        "expected tb to stop on SIGINT (exit 130 or signal), got {status:?}"
    );
}

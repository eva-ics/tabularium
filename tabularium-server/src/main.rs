//! `tabularium-server` — Axum REST + JSON-RPC for the document store.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use tabularium_server::otel::{self, Telemetry};
use tabularium_server::{config, web};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(name = "tabularium-server", version, about)]
struct Cli {
    /// Path to the TOML configuration file
    #[arg(
        short = 'c',
        long = "config",
        default_value = "/etc/tabularium/config.toml"
    )]
    config: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let telemetry = otel::init_tracing();

    let cli = Cli::parse();
    let config_path = std::env::var_os("TABULARIUM_CONFIG")
        .and_then(|s| {
            let t = s.to_string_lossy();
            let t = t.trim();
            if t.is_empty() {
                None
            } else {
                Some(PathBuf::from(t))
            }
        })
        .unwrap_or(cli.config);
    let cfg = config::load(&config_path)?;

    if let Some(parent) = cfg.server.database_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir_all(&cfg.server.index_dir)?;

    let worker_threads = if cfg.server.workers == 0 {
        std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(1)
    } else {
        cfg.server.workers as usize
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .enable_all()
        .build()?;

    let result = rt.block_on(run(cfg, &telemetry));
    telemetry.shutdown();
    result
}

async fn run(
    cfg: config::Config,
    telemetry: &Telemetry,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    otel::maybe_spawn_runtime_metrics(telemetry);
    let process_started_at = bma_ts::Monotonic::now();

    #[cfg(feature = "mcp")]
    let mcp_listen = cfg
        .mcp
        .as_ref()
        .and_then(|m| m.listen.as_ref().map(ToString::to_string));
    #[cfg(feature = "mcp")]
    let mcp_server_help_path = cfg.mcp.as_ref().and_then(|m| m.server_help.clone());
    #[cfg(feature = "mcp")]
    let mcp_full = cfg.mcp.as_ref().is_some_and(|m| m.full);

    let db_uri = format!("sqlite://{}", cfg.server.database_path.display());
    let mut db = tabularium::SqliteDatabase::init(&db_uri, &cfg.server.index_dir, 256).await?;
    if let Some(semantic) = cfg.semantic {
        db = db
            .with_semantic(
                semantic,
                &cfg.server.index_dir.with_extension("semantic.index"),
            )
            .await?;
    }
    let db = Arc::new(db);
    db.start_semantic_worker();
    let wait_timeout = Duration::from_secs(cfg.server.timeout.max(1));
    let oidc = match cfg.oidc.as_ref() {
        None => None,
        Some(sec) => Some(std::sync::Arc::new(
            tabularium_server::jwt_assertion::AssertionRuntime::bootstrap(sec.clone()).await?,
        )),
    };
    let anonymous = cfg
        .anonymous
        .as_ref()
        .map(|a| tabularium::AuthContext::new("anonymous".into(), a.body.clone()));
    let app_state = web::AppState {
        db,
        wait_timeout,
        process_started_at,
        authenticate_api: cfg.server.authenticate,
        authenticate_mcp: cfg.mcp.as_ref().is_some_and(|m| m.authenticate),
        oidc,
        anonymous,
    };
    let app = web::router(app_state.clone());

    // One tree: Ctrl+C cancels MCP streamable HTTP (child) and triggers graceful REST shutdown.
    let shutdown = CancellationToken::new();

    #[cfg(feature = "mcp")]
    if let Some(listen) = mcp_listen {
        let server_help = match mcp_server_help_path {
            None => String::new(),
            Some(p) => std::fs::read_to_string(&p)
                .map_err(|e| format!("mcp.server_help {}: {e}", p.display()))?,
        };
        let st = app_state.clone();
        let mcp_cancel = shutdown.child_token();
        tokio::spawn(async move {
            if let Err(e) =
                tabularium_server::mcp::serve(&listen, st, server_help, mcp_full, mcp_cancel).await
            {
                tracing::error!(error = %e, "MCP server exited with error");
            }
        });
    }

    let shutdown_sig = shutdown.clone();
    #[cfg(unix)]
    tokio::spawn({
        let shutdown_sig = shutdown_sig.clone();
        async move {
            use tokio::signal::unix::{SignalKind, signal};
            let mut term = match signal(SignalKind::terminate()) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "SIGTERM listener failed");
                    return;
                }
            };
            if term.recv().await.is_some() {
                tracing::info!("SIGTERM received; closing listeners for access control");
                shutdown_sig.cancel();
            }
        }
    });

    tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                tracing::info!("shutdown signal received; closing listeners for access control");
                shutdown_sig.cancel();
            }
            Err(e) => tracing::warn!(error = %e, "ctrl_c listener failed"),
        }
    });

    let listener = TcpListener::bind(&cfg.server.listen).await?;
    tracing::info!(listen = %cfg.server.listen, "tabularium-server listening");
    axum::serve(listener, app)
        .with_graceful_shutdown({
            let shutdown = shutdown.clone();
            async move {
                shutdown.cancelled().await;
            }
        })
        .await?;
    Ok(())
}

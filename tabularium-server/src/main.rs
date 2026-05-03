//! `tabularium-server` — Axum REST + JSON-RPC for the librarium.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use tabularium_server::{config, web};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::format::Format;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Buffers output until newline, then writes the line with leading whitespace trimmed (for systemd logs).
struct LineTrimWriter<W: std::io::Write> {
    inner: W,
    buf: Vec<u8>,
}

impl<W: std::io::Write> LineTrimWriter<W> {
    fn new(inner: W) -> Self {
        Self {
            inner,
            buf: Vec::new(),
        }
    }

    fn flush_line(&mut self) -> std::io::Result<()> {
        if let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
            let line = &self.buf[..=i];
            let s = String::from_utf8_lossy(line);
            let trimmed = s.trim_start();
            self.inner.write_all(trimmed.as_bytes())?;
            self.buf.drain(..=i);
        }
        Ok(())
    }
}

impl<W: std::io::Write> std::io::Write for LineTrimWriter<W> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(b);
        self.flush_line()?;
        Ok(b.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if !self.buf.is_empty() {
            let s = String::from_utf8_lossy(&self.buf);
            let trimmed = s.trim_start();
            self.inner.write_all(trimmed.as_bytes())?;
            self.buf.clear();
        }
        self.inner.flush()
    }
}

/// [`tracing_subscriber::fmt::MakeWriter`] when `INVOCATION_ID` is set; wraps stdout in [`LineTrimWriter`].
struct SystemdWriter;

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SystemdWriter {
    type Writer = LineTrimWriter<std::io::Stdout>;

    fn make_writer(&'a self) -> Self::Writer {
        LineTrimWriter::new(std::io::stdout())
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,tabularium=debug,tabularium_server=debug,tantivy=warn")
    });
    let is_systemd = std::env::var("INVOCATION_ID").is_ok();

    if is_systemd {
        tracing_subscriber::registry()
            .with(filter)
            .with(
                fmt::layer()
                    .with_writer(SystemdWriter)
                    .with_ansi(false)
                    .event_format(Format::default().without_time())
                    .with_target(true)
                    .with_level(true),
            )
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(
                fmt::layer()
                    .with_writer(std::io::stdout)
                    .with_ansi(true)
                    .with_target(true)
                    .with_level(true),
            )
            .init();
    }
}

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
    init_tracing();

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

    rt.block_on(run(cfg))
}

async fn run(cfg: config::Config) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
    let db = Arc::new(tabularium::SqliteDatabase::init(&db_uri, &cfg.server.index_dir, 256).await?);
    let wait_timeout = Duration::from_secs(cfg.server.timeout.max(1));
    let app_state = web::AppState {
        db,
        wait_timeout,
        process_started_at,
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
                tracing::info!("SIGTERM received; closing listeners for the Throne");
                shutdown_sig.cancel();
            }
        }
    });

    tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                tracing::info!("shutdown signal received; closing listeners for the Throne");
                shutdown_sig.cancel();
            }
            Err(e) => tracing::warn!(error = %e, "ctrl_c listener failed"),
        }
    });

    let listener = TcpListener::bind(&cfg.server.listen).await?;
    tracing::info!(listen = %cfg.server.listen, "tabularium-server bound; praise the Omnissiah");
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

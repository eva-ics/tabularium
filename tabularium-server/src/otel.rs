//! OpenTelemetry (OTLP/HTTP protobuf) layered onto the existing `tracing` subscriber.
//!
//! Export is **opt-in** via environment variables. When no OTLP endpoint is set (or
//! `OTEL_SDK_DISABLED=true`), only the fmt subscriber is installed and the process
//! behaves as before.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use opentelemetry::KeyValue;
use opentelemetry::global;
use opentelemetry::metrics::{Counter, Histogram, MeterProvider as _, UpDownCounter};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{MetricExporter, Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::SdkTracerProvider;
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

static MCP_METRICS: OnceLock<McpMetrics> = OnceLock::new();

/// MCP tool-call instruments (set when OTLP export is enabled).
#[derive(Clone)]
pub struct McpMetrics {
    /// Tool call duration in seconds.
    pub duration: Histogram<f64>,
    /// Tool invocations (success and failure).
    pub calls: Counter<u64>,
    /// Application-level tool errors (`Err` from tool handlers).
    pub errors: Counter<u64>,
    /// In-flight tool calls.
    pub active: UpDownCounter<i64>,
    /// Explicit wait / long-poll timeouts.
    pub timeouts: Counter<u64>,
}

/// OpenTelemetry providers to flush on process shutdown.
pub struct Telemetry {
    tracer_provider: Option<SdkTracerProvider>,
    meter_provider: Option<SdkMeterProvider>,
}

impl Telemetry {
    /// Disabled telemetry handle (fmt logging only, no OTLP exporters).
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            tracer_provider: None,
            meter_provider: None,
        }
    }

    /// Indicates whether OTLP export is active.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.tracer_provider.is_some()
    }

    /// Shuts down exporters on a helper thread so the Tokio runtime is not blocked indefinitely.
    pub fn shutdown(self) {
        const LIMIT: Duration = Duration::from_secs(5);
        if let Some(provider) = self.tracer_provider {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let r = provider.shutdown();
                let _ = tx.send(r);
            });
            match rx.recv_timeout(LIMIT) {
                Ok(Ok(())) => {}
                Ok(Err(e)) => eprintln!("OpenTelemetry tracer shutdown failed: {e}"),
                Err(_) => eprintln!("OpenTelemetry tracer shutdown timed out after {LIMIT:?}"),
            }
        }
        if let Some(provider) = self.meter_provider {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let r = provider.shutdown();
                let _ = tx.send(r);
            });
            match rx.recv_timeout(LIMIT) {
                Ok(Ok(())) => {}
                Ok(Err(e)) => eprintln!("OpenTelemetry meter shutdown failed: {e}"),
                Err(_) => eprintln!("OpenTelemetry meter shutdown timed out after {LIMIT:?}"),
            }
        }
    }
}

/// Returns MCP metrics when OTLP was initialized.
#[must_use]
pub fn mcp_metrics() -> Option<&'static McpMetrics> {
    MCP_METRICS.get()
}

/// Decrements the active-tool gauge on drop.
pub struct ActiveToolGuard {
    tool: String,
}

impl Drop for ActiveToolGuard {
    fn drop(&mut self) {
        if let Some(m) = mcp_metrics() {
            m.active
                .add(-1, &[KeyValue::new("mcp.tool", self.tool.clone())]);
        }
    }
}

/// Increments active tool count until the returned guard is dropped.
#[must_use]
pub fn enter_active_tool(tool: &str) -> Option<ActiveToolGuard> {
    let m = mcp_metrics()?;
    m.active
        .add(1, &[KeyValue::new("mcp.tool", tool.to_owned())]);
    Some(ActiveToolGuard {
        tool: tool.to_owned(),
    })
}

/// Records duration / call / error counters for a finished MCP tool invocation.
pub fn record_tool_call(tool: &str, started: Instant, ok: bool) {
    let Some(m) = mcp_metrics() else {
        return;
    };
    let attrs = [KeyValue::new("mcp.tool", tool.to_owned())];
    m.duration.record(started.elapsed().as_secs_f64(), &attrs);
    m.calls.add(1, &attrs);
    if !ok {
        m.errors.add(1, &attrs);
    }
}

/// Records an explicit wait / long-poll timeout.
pub fn record_wait_timeout() {
    if let Some(m) = mcp_metrics() {
        m.timeouts.add(1, &[]);
    }
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn sdk_disabled() -> bool {
    matches!(
        env_nonempty("OTEL_SDK_DISABLED").as_deref(),
        Some("true" | "1" | "TRUE" | "True")
    )
}

/// Indicates whether OTLP should be enabled from the process environment.
#[must_use]
pub fn otlp_configured() -> bool {
    if sdk_disabled() {
        return false;
    }
    env_nonempty("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT").is_some()
        || env_nonempty("OTEL_EXPORTER_OTLP_ENDPOINT").is_some()
}

fn traces_protocol() -> Protocol {
    let raw = env_nonempty("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL")
        .or_else(|| env_nonempty("OTEL_EXPORTER_OTLP_PROTOCOL"))
        .unwrap_or_else(|| "http/protobuf".into());
    match raw.as_str() {
        "http/protobuf" | "http/proto" => Protocol::HttpBinary,
        other => {
            eprintln!(
                "Unsupported OTEL protocol `{other}`; using http/protobuf (this build exports OTLP/HTTP protobuf only)"
            );
            Protocol::HttpBinary
        }
    }
}

fn default_env_filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,tabularium=debug,tabularium_server=debug,tantivy=warn")
    })
}

fn install_fmt_only(filter: EnvFilter) {
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

fn build_resource() -> Resource {
    let service = env_nonempty("OTEL_SERVICE_NAME").unwrap_or_else(|| "tabularium".into());
    Resource::builder().with_service_name(service).build()
}

fn init_otlp_providers() -> Result<(SdkTracerProvider, SdkMeterProvider), String> {
    global::set_text_map_propagator(TraceContextPropagator::new());
    let resource = build_resource();
    let protocol = traces_protocol();

    let span_exporter = SpanExporter::builder()
        .with_http()
        .with_protocol(protocol)
        .build()
        .map_err(|e| format!("OTLP span exporter: {e}"))?;

    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(span_exporter)
        .with_resource(resource.clone())
        .build();

    let metric_exporter = MetricExporter::builder()
        .with_http()
        .with_protocol(protocol)
        .build()
        .map_err(|e| format!("OTLP metric exporter: {e}"))?;

    let meter_provider = SdkMeterProvider::builder()
        .with_periodic_exporter(metric_exporter)
        .with_resource(resource)
        .build();

    Ok((tracer_provider, meter_provider))
}

fn install_mcp_instruments(meter_provider: &SdkMeterProvider) {
    let meter = meter_provider.meter("tabularium-server");
    let metrics = McpMetrics {
        duration: meter
            .f64_histogram("mcp.client.operation.duration")
            .with_description("MCP tool call duration in seconds")
            .with_unit("s")
            .build(),
        calls: meter
            .u64_counter("mcp.tool.calls")
            .with_description("MCP tool invocations")
            .build(),
        errors: meter
            .u64_counter("mcp.tool.errors")
            .with_description("MCP tool application-level errors")
            .build(),
        active: meter
            .i64_up_down_counter("mcp.tool.active")
            .with_description("In-flight MCP tool calls")
            .build(),
        timeouts: meter
            .u64_counter("mcp.wait.timeouts")
            .with_description("Document wait / long-poll timeouts")
            .build(),
    };
    let _ = MCP_METRICS.set(metrics);
}

/// Publishes stable Tokio runtime gauges every few seconds while OTLP metrics are enabled.
pub fn spawn_tokio_runtime_metrics(meter_provider: &SdkMeterProvider) {
    let meter = meter_provider.meter("tabularium-server");
    let workers = meter
        .u64_gauge("tokio.runtime.workers")
        .with_description("Tokio runtime worker thread count")
        .build();
    let alive = meter
        .u64_gauge("tokio.runtime.alive_tasks")
        .with_description("Tokio alive tasks")
        .build();
    let queue = meter
        .u64_gauge("tokio.runtime.global_queue_depth")
        .with_description("Tokio global injection queue depth")
        .build();

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(5));
        loop {
            ticker.tick().await;
            let metrics = tokio::runtime::Handle::current().metrics();
            workers.record(metrics.num_workers() as u64, &[]);
            alive.record(metrics.num_alive_tasks() as u64, &[]);
            queue.record(metrics.global_queue_depth() as u64, &[]);
        }
    });
}

/// Installs the tracing subscriber; enables OTLP when configured via env.
pub fn init_tracing() -> Telemetry {
    let filter = default_env_filter();

    if !otlp_configured() {
        install_fmt_only(filter);
        return Telemetry::disabled();
    }

    let (tracer_provider, meter_provider) = match init_otlp_providers() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("OpenTelemetry init failed ({e}); continuing with fmt logging only");
            install_fmt_only(filter);
            return Telemetry::disabled();
        }
    };

    global::set_tracer_provider(tracer_provider.clone());
    global::set_meter_provider(meter_provider.clone());
    install_mcp_instruments(&meter_provider);

    let is_systemd = std::env::var("INVOCATION_ID").is_ok();
    // Build the OTel layer inside each branch so its `Layer` impl matches the fmt writer type.
    if is_systemd {
        let otel_layer =
            tracing_opentelemetry::layer().with_tracer(tracer_provider.tracer("tabularium-server"));
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
            .with(otel_layer)
            .init();
    } else {
        let otel_layer =
            tracing_opentelemetry::layer().with_tracer(tracer_provider.tracer("tabularium-server"));
        tracing_subscriber::registry()
            .with(filter)
            .with(
                fmt::layer()
                    .with_writer(std::io::stdout)
                    .with_ansi(true)
                    .with_target(true)
                    .with_level(true),
            )
            .with(otel_layer)
            .init();
    }

    Telemetry {
        tracer_provider: Some(tracer_provider),
        meter_provider: Some(meter_provider),
    }
}

/// Starts Tokio runtime metric export when telemetry is enabled.
pub fn maybe_spawn_runtime_metrics(telemetry: &Telemetry) {
    if let Some(mp) = telemetry.meter_provider.as_ref() {
        spawn_tokio_runtime_metrics(mp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvVarGuard {
        key: &'static str,
        prev: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, val: &str) -> Self {
            let prev = std::env::var_os(key);
            // Safety: guarded by ENV_LOCK in each test.
            unsafe { std::env::set_var(key, val) };
            Self { key, prev }
        }

        fn unset(key: &'static str) -> Self {
            let prev = std::env::var_os(key);
            unsafe { std::env::remove_var(key) };
            Self { key, prev }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => unsafe { std::env::set_var(self.key, v) },
                None => unsafe { std::env::remove_var(self.key) },
            }
        }
    }

    #[test]
    fn otlp_configured_false_without_endpoint() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _a = EnvVarGuard::unset("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT");
        let _b = EnvVarGuard::unset("OTEL_EXPORTER_OTLP_ENDPOINT");
        let _c = EnvVarGuard::unset("OTEL_SDK_DISABLED");
        assert!(!otlp_configured());
    }

    #[test]
    fn otlp_configured_respects_sdk_disabled() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _ep = EnvVarGuard::set("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:4318");
        let _dis = EnvVarGuard::set("OTEL_SDK_DISABLED", "true");
        assert!(!otlp_configured());
    }

    #[test]
    fn otlp_configured_true_with_traces_endpoint() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _dis = EnvVarGuard::unset("OTEL_SDK_DISABLED");
        let _ep = EnvVarGuard::set(
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            "http://127.0.0.1:5080/api/default/v1/traces",
        );
        assert!(otlp_configured());
    }
}

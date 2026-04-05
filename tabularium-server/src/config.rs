//! TOML configuration — `#[serde(deny_unknown_fields)]` per project scrolls.

use std::path::Path;

use serde::Deserialize;

use tabularium::{Error, Result};

/// Root config file shape (`config.toml`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub server: ServerSection,
    #[serde(default)]
    pub mcp: Option<McpSection>,
}

/// Optional `[mcp]` — streamable HTTP MCP (`/mcp`); omit `listen` to keep MCP dormant.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpSection {
    /// e.g. `127.0.0.1:3031`; when unset, the MCP listener is not started.
    #[serde(default)]
    pub listen: Option<String>,
    /// Deployment-specific help text (read at startup).
    #[serde(default)]
    pub server_help: Option<std::path::PathBuf>,
}

fn default_timeout_secs() -> u64 {
    3600
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerSection {
    /// `host:port`, e.g. `127.0.0.1:3050`.
    pub listen: String,
    pub database_path: std::path::PathBuf,
    pub index_dir: std::path::PathBuf,
    /// Tokio worker threads; `0` means `available_parallelism()`.
    pub workers: u32,
    /// Long-poll `wait` / `?wait=true` ceiling (seconds); default 3600.
    #[serde(default = "default_timeout_secs")]
    pub timeout: u64,
}

pub fn load(path: &Path) -> Result<Config> {
    let raw = std::fs::read_to_string(path).map_err(|e| Error::Io(e.to_string()))?;
    toml::from_str(&raw).map_err(|e| Error::InvalidInput(e.to_string()))
}

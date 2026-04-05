//! Tabularium — document storage for the faithful (SQLite + full-text search).
//!
//! Cargo features (see `Cargo.toml`):
//! - **`db`** — `Database`, `SqliteStorage`, Tantivy search, SQLite.
//! - **`client`** — `rpc::Client` (`reqwest`); implies **`db`**.
//! - Default: **`client`** (implies **`db`**; same as **`full`**).
//!
//! Binaries in workspace crates enable the features they need explicitly.

#![forbid(unsafe_code)]

mod error;
pub mod jsonrpc_codes;
mod validation;

#[cfg(feature = "client")]
pub mod client_headers;
#[cfg(feature = "db")]
pub mod db;
pub mod resource_path;
#[cfg(feature = "client")]
pub mod rpc;
#[cfg(feature = "db")]
pub mod text_lines;
#[cfg(feature = "client")]
pub mod ws;

#[cfg(feature = "db")]
pub use bma_ts::Timestamp;
#[cfg(feature = "client")]
pub use client_headers::{
    header_map_from_lines, header_map_redacted_summary, merge_header_line, merge_into,
    parse_header_line, parse_tb_headers_env,
};
#[cfg(feature = "db")]
pub use db::parse_user_timestamp;
#[cfg(feature = "db")]
pub use db::{
    Database, DocumentMeta, DocumentWaitStatus, EntryId, EntryKind, GrepLine, ListedEntry,
    SearchHit, SqliteDatabase, SqliteStorage, Storage, WcStats,
};
pub use error::{Error, Result};
#[cfg(feature = "client")]
pub use reqwest::header::HeaderMap;
#[cfg(feature = "db")]
pub use text_lines::TailMode;
pub use validation::{validate_chat_speaker_id, validate_entity_name};

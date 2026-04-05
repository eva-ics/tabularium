//! Library surface for tests and future embedding.

pub mod config;
mod embedded_ui;
#[cfg(feature = "mcp")]
pub mod mcp;
mod multipart_body;
mod rpc_preview;
pub mod web;
pub mod ws_doc;

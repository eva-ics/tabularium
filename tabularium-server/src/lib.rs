//! Library surface for tests and future embedding.

pub(crate) mod auth;
pub mod config;
mod embedded_ui;
#[cfg(feature = "mcp")]
pub mod mcp;
mod multipart_body;
mod rpc_preview;
mod test_payload;
pub mod web;
pub mod ws_doc;

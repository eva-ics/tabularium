//! WebSocket client (`ws://` only in stage 1; `wss://` deferred).

mod client;

pub use client::{Client, RecvMessage, ws_url_from_http_base};

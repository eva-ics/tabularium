//! JSON-RPC client (`reqwest`) for tabularium servers.
//!
//! This module is only available with the **`client`** Cargo feature.
//!
//! ## Application error codes
//!
//! See [`crate::jsonrpc_codes`] (e.g. [`crate::jsonrpc_codes::DUPLICATE_RESOURCE`] for conflicts).

mod client;

pub use client::{
    Client, DocumentBody, DocumentMetaRow, GrepLineRow, ListedEntryRow, SearchHitRow, ServerTest,
    StatRow, WcRow,
};

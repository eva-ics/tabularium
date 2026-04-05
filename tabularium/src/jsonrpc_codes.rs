//! Stable JSON-RPC 2.0 `error.code` values returned by tabularium servers.
//!
//! Used by `tabularium-server` and the `rpc::Client` so upsert and other logic do not
//! depend on parsing English `error.message` strings.

/// Document or directory name already exists ([`crate::Error::Duplicate`]).
pub const DUPLICATE_RESOURCE: i32 = -32002;

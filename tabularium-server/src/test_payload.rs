use serde_json::{Value, json};

pub(crate) fn test_payload(
    process_started_at: bma_ts::Monotonic,
    authenticate_api: bool,
    oidc_enabled: bool,
) -> Value {
    let nanos = process_started_at.elapsed().as_nanos();
    let uptime = u64::try_from(nanos).unwrap_or(u64::MAX);
    json!({
        "product_name": "tabularium",
        "product_version": env!("CARGO_PKG_VERSION"),
        "uptime": uptime,
        "authenticate_api": authenticate_api,
        "oidc_enabled": oidc_enabled,
    })
}

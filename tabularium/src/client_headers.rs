//! Optional extra HTTP headers for outbound RPC and WebSocket clients (WAF / reverse proxy).

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use crate::{Error, Result};

fn is_sensitive_header(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "authorization" | "cookie" | "set-cookie" | "proxy-authorization"
    )
}

/// Split `Name: value` on the first colon (value may contain further colons).
pub fn parse_header_line(line: &str) -> Result<(String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Err(Error::InvalidInput(
            "header line must be non-empty `Name: value`".into(),
        ));
    }
    let Some((name, rest)) = line.split_once(':') else {
        return Err(Error::InvalidInput(format!(
            "header line must contain ':': {line:?}"
        )));
    };
    let name = name.trim();
    let value = rest.trim();
    if name.is_empty() {
        return Err(Error::InvalidInput("empty header name".into()));
    }
    Ok((name.to_string(), value.to_string()))
}

/// Parse `Name: value` and insert into the map (replaces an existing same name).
pub fn merge_header_line(map: &mut HeaderMap, line: &str) -> Result<()> {
    let (n, v) = parse_header_line(line)?;
    insert_pair(map, &n, &v)
}

fn insert_pair(map: &mut HeaderMap, name: &str, value: &str) -> Result<()> {
    let hn = HeaderName::from_bytes(name.as_bytes())
        .map_err(|e| Error::InvalidInput(format!("invalid header name {name:?}: {e}")))?;
    let hv = HeaderValue::from_str(value).map_err(|e| {
        Error::InvalidInput(format!(
            "invalid header value for {name:?} (use visible ASCII or encode per RFC 9110): {e}"
        ))
    })?;
    map.insert(hn, hv);
    Ok(())
}

/// Parse `TB_HEADERS`: one `Name: value` per line (`\n` or `\r\n`). Empty lines and `#` comments skipped.
/// Commas inside values are fine; newlines inside a value are not supported (use another line for another header).
pub fn parse_tb_headers_env(raw: &str) -> Result<HeaderMap> {
    let mut map = HeaderMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (n, v) = parse_header_line(line)?;
        insert_pair(&mut map, &n, &v)?;
    }
    Ok(map)
}

/// Build a map from independent lines (config file array entries, each `Name: value`).
pub fn header_map_from_lines(
    lines: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<HeaderMap> {
    let mut map = HeaderMap::new();
    for line in lines {
        let line = line.as_ref().trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (n, v) = parse_header_line(line)?;
        insert_pair(&mut map, &n, &v)?;
    }
    Ok(map)
}

/// Merge into `into_map`: each pair from `source` is inserted (same name replaced).
pub fn merge_into(into_map: &mut HeaderMap, source: HeaderMap) {
    for (maybe_name, v) in source {
        if let Some(name) = maybe_name {
            into_map.insert(name, v);
        }
    }
}

/// Comma-separated summary for logs — sensitive header values redacted.
pub fn header_map_redacted_summary(map: &HeaderMap) -> String {
    let mut parts = Vec::new();
    for (name, v) in map {
        let disp = if is_sensitive_header(name.as_str()) {
            "<redacted>".to_string()
        } else {
            String::from_utf8_lossy(v.as_bytes()).to_string()
        };
        parts.push(format!("{}: {disp}", name.as_str()));
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tb_headers_newlines_multi() {
        let m = parse_tb_headers_env("Authorization: Bearer x\nX-Trace: abc").unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(
            m.get("authorization").and_then(|v| v.to_str().ok()),
            Some("Bearer x")
        );
    }

    #[test]
    fn colon_in_value_preserved() {
        let m = parse_tb_headers_env("X: a:b:c").unwrap();
        assert_eq!(m.get("x").and_then(|v| v.to_str().ok()), Some("a:b:c"));
    }

    #[test]
    fn redacted_summary_hides_authorization() {
        let mut m = HeaderMap::new();
        m.insert(
            HeaderName::from_static("authorization"),
            HeaderValue::from_static("Bearer SECRET"),
        );
        m.insert(
            HeaderName::from_static("x-ok"),
            HeaderValue::from_static("visible"),
        );
        let s = header_map_redacted_summary(&m);
        assert!(s.contains("authorization: <redacted>"));
        assert!(s.contains("x-ok: visible"));
        assert!(!s.contains("SECRET"));
    }
}

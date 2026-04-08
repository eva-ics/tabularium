//! Compact JSON-RPC params for transport-level request logs (truncate bulk fields).

use std::fmt::Write as _;

use serde_json::Value;

const STR_CLIP_CHARS: usize = 56;
const OUTPUT_CAP: usize = 512;

/// Single-line-ish preview of JSON params for `info!` (truncates long strings and total output).
pub(crate) fn format_rpc_params_preview(params: Option<&Value>) -> String {
    let Some(v) = params else {
        return String::from("{}");
    };
    let mut out = String::new();
    append_value(v, &mut out, STR_CLIP_CHARS, OUTPUT_CAP);
    out
}

fn append_value(v: &Value, out: &mut String, clip: usize, cap: usize) {
    if out.len() >= cap {
        return;
    }
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            let _ = write!(out, "{n}");
        }
        Value::String(s) => append_str(s, out, clip, cap),
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if out.len() >= cap {
                    break;
                }
                if i > 0 {
                    out.push(',');
                }
                append_value(x, out, clip, cap);
            }
            out.push(']');
        }
        Value::Object(m) => {
            out.push('{');
            let mut first = true;
            for (k, val) in m {
                if out.len() >= cap {
                    let _ = write!(out, "…");
                    break;
                }
                if !first {
                    out.push(',');
                }
                first = false;
                out.push_str(k);
                out.push(':');
                append_value(val, out, clip, cap);
            }
            out.push('}');
        }
    }
}

/// Collapses runs of CR/LF (`\r`, `\n`, and mixtures) into at most one ASCII space (log hygiene only).
fn sanitize_newlines_for_log(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for ch in s.chars() {
        if ch == '\r' || ch == '\n' {
            pending_space = true;
        } else {
            if pending_space {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
                pending_space = false;
            }
            out.push(ch);
        }
    }
    if pending_space && !out.is_empty() && !out.ends_with(' ') {
        out.push(' ');
    }
    out
}

fn append_str(s: &str, out: &mut String, clip: usize, cap: usize) {
    let budget = cap.saturating_sub(out.len());
    if budget <= 3 {
        return;
    }
    let n = s.chars().count();
    let shown = sanitize_newlines_for_log(&s.chars().take(clip).collect::<String>());
    let mut piece = if n > clip {
        format!("{}…({n} chars)", shown)
    } else {
        shown
    };
    while piece.len() > budget {
        piece.pop();
    }
    if piece.is_empty() {
        piece.push('…');
    }
    out.push_str(&piece);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn preview_replaces_embedded_newlines_with_space() {
        let v = json!({ "p": "a\nb" });
        let s = format_rpc_params_preview(Some(&v));
        assert!(!s.contains(['\r', '\n']));
        assert_eq!(s, "{p:a b}");
    }

    #[test]
    fn preview_replaces_crlf() {
        let v = json!({ "p": "a\r\nb" });
        let s = format_rpc_params_preview(Some(&v));
        assert!(!s.contains(['\r', '\n']));
        assert_eq!(s, "{p:a b}");
    }

    #[test]
    fn preview_replaces_lone_cr() {
        let v = json!({ "p": "a\rb" });
        let s = format_rpc_params_preview(Some(&v));
        assert!(!s.contains(['\r', '\n']));
        assert_eq!(s, "{p:a b}");
    }

    #[test]
    fn preview_replaces_nr_mixture() {
        let v = json!({ "p": "a\n\rb" });
        let s = format_rpc_params_preview(Some(&v));
        assert!(!s.contains(['\r', '\n']));
        assert_eq!(s, "{p:a b}");
    }

    #[test]
    fn preview_truncates_then_sanitizes_no_raw_newline_in_logged_prefix() {
        let xs = "x".repeat(55);
        let s_body = format!("{xs}\ny");
        assert!(s_body.chars().count() > STR_CLIP_CHARS);
        let v = json!({ "p": s_body });
        let s = format_rpc_params_preview(Some(&v));
        assert!(!s.contains(['\r', '\n']));
        assert!(s.contains('…'));
        assert!(s.contains(" chars)"));
    }

    #[test]
    fn preview_respects_output_cap() {
        let mut m = serde_json::Map::new();
        for i in 0..80 {
            m.insert(format!("k{i}"), Value::String("v".into()));
        }
        let v = Value::Object(m);
        let s = format_rpc_params_preview(Some(&v));
        assert!(!s.contains(['\r', '\n']));
        assert!(s.len() <= OUTPUT_CAP, "len {}", s.len());
    }
}

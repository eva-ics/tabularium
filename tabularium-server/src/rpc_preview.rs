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

fn append_str(s: &str, out: &mut String, clip: usize, cap: usize) {
    let budget = cap.saturating_sub(out.len());
    if budget <= 3 {
        return;
    }
    let n = s.chars().count();
    let shown: String = s.chars().take(clip).collect();
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

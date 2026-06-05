//! Word-count metrics filter and JSON shapes for `wc` RPC/CLI.

use serde_json::{Map, Value, json};

use crate::db::WcStats;

/// Which `wc` metrics to include in RPC/CLI output.
#[derive(Debug, Clone, Copy, Default)]
pub struct WcMetricsFilter {
    pub lines: bool,
    pub words: bool,
    pub bytes: bool,
    pub chars: bool,
}

impl WcMetricsFilter {
    pub const fn all() -> Self {
        Self {
            lines: true,
            words: true,
            bytes: true,
            chars: true,
        }
    }

    pub const fn posix_default() -> Self {
        Self {
            lines: true,
            words: true,
            bytes: true,
            chars: false,
        }
    }

    pub fn from_rpc_params(m: &Map<String, Value>) -> Self {
        let lines = m.get("lines").and_then(Value::as_bool);
        let words = m.get("words").and_then(Value::as_bool);
        let bytes = m.get("bytes").and_then(Value::as_bool);
        let chars = m.get("chars").and_then(Value::as_bool);
        let any = [lines, words, bytes, chars]
            .into_iter()
            .any(|o| o == Some(true));
        if !any {
            return Self::all();
        }
        Self {
            lines: lines == Some(true),
            words: words == Some(true),
            bytes: bytes == Some(true),
            chars: chars == Some(true),
        }
    }

    pub fn from_cli_flags(lines: bool, words: bool, bytes: bool, chars: bool) -> Self {
        if !lines && !words && !bytes && !chars {
            return Self::posix_default();
        }
        Self {
            lines,
            words,
            bytes,
            chars,
        }
    }

    pub fn active_columns(&self) -> usize {
        usize::from(self.lines)
            + usize::from(self.words)
            + usize::from(self.bytes)
            + usize::from(self.chars)
    }

    pub fn column_labels(&self) -> Vec<&'static str> {
        let mut cols = Vec::with_capacity(self.active_columns());
        if self.lines {
            cols.push("lines");
        }
        if self.words {
            cols.push("words");
        }
        if self.bytes {
            cols.push("bytes");
        }
        if self.chars {
            cols.push("chars");
        }
        cols
    }

    pub fn format_counts(&self, stats: &WcStats) -> Vec<String> {
        self.format_optional_counts(
            Some(stats.lines()),
            Some(stats.words()),
            Some(stats.bytes()),
            Some(stats.chars()),
        )
    }

    pub fn format_optional_counts(
        &self,
        lines: Option<usize>,
        words: Option<usize>,
        bytes: Option<u64>,
        chars: Option<usize>,
    ) -> Vec<String> {
        let mut cols = Vec::with_capacity(self.active_columns());
        if self.lines {
            cols.push(lines.unwrap_or(0).to_string());
        }
        if self.words {
            cols.push(words.unwrap_or(0).to_string());
        }
        if self.bytes {
            cols.push(bytes.unwrap_or(0).to_string());
        }
        if self.chars {
            cols.push(chars.unwrap_or(0).to_string());
        }
        cols
    }
}

fn wc_metrics_object(stats: &WcStats, filter: &WcMetricsFilter) -> Map<String, Value> {
    let mut m = Map::new();
    if filter.lines {
        m.insert("lines".into(), json!(stats.lines()));
    }
    if filter.words {
        m.insert("words".into(), json!(stats.words()));
    }
    if filter.bytes {
        m.insert("bytes".into(), json!(stats.bytes()));
    }
    if filter.chars {
        m.insert("chars".into(), json!(stats.chars()));
    }
    m
}

pub fn wc_stats_to_value(stats: &WcStats, filter: &WcMetricsFilter) -> Value {
    Value::Object(wc_metrics_object(stats, filter))
}

pub fn wc_file_to_value(path: &str, stats: &WcStats, filter: &WcMetricsFilter) -> Value {
    let mut m = wc_metrics_object(stats, filter);
    m.insert("path".into(), json!(path));
    Value::Object(m)
}

pub fn sum_wc_stats(rows: &[WcStats]) -> WcStats {
    let mut bytes = 0u64;
    let mut lines = 0usize;
    let mut words = 0usize;
    let mut chars = 0usize;
    for w in rows {
        bytes += w.bytes();
        lines += w.lines();
        words += w.words();
        chars += w.chars();
    }
    WcStats::from_totals(bytes, lines, words, chars)
}

#[cfg(test)]
mod tests {
    use super::{WcMetricsFilter, wc_stats_to_value};
    use crate::db::WcStats;
    use serde_json::json;

    #[test]
    fn from_rpc_params_defaults_to_all_when_no_flags() {
        let f = WcMetricsFilter::from_rpc_params(&serde_json::Map::new());
        assert!(f.lines && f.words && f.bytes && f.chars);
    }

    #[test]
    fn from_rpc_params_honours_true_flags_only() {
        let mut m = serde_json::Map::new();
        m.insert("lines".into(), json!(true));
        m.insert("words".into(), json!(false));
        let f = WcMetricsFilter::from_rpc_params(&m);
        assert!(f.lines);
        assert!(!f.words);
    }

    #[test]
    fn wc_stats_to_value_respects_filter() {
        let stats = WcStats::from_totals(10, 2, 3, 8);
        let f = WcMetricsFilter {
            lines: true,
            words: false,
            bytes: false,
            chars: false,
        };
        assert_eq!(wc_stats_to_value(&stats, &f), json!({ "lines": 2 }));
    }
}

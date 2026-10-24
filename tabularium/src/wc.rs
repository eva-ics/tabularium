//! Word-count metrics filter and JSON shapes for `wc` RPC/CLI.

use serde_json::{Map, Value, json};

use crate::db::WcStats;

/// Which `wc` metrics to include in RPC/CLI output.
#[derive(Debug, Clone, Copy, Default)]
pub struct WcMetricsFilter {
    /// Indicator whether line counts are included.
    pub lines: bool,

    /// Indicator whether word counts are included.
    pub words: bool,

    /// Indicator whether byte counts are included.
    pub bytes: bool,

    /// Indicator whether character counts are included.
    pub chars: bool,
}

impl WcMetricsFilter {
    /// Returns a filter that includes every metric.
    pub const fn all() -> Self {
        Self {
            lines: true,
            words: true,
            bytes: true,
            chars: true,
        }
    }

    /// Returns the GNU/`wc` default columns: lines, words, bytes.
    pub const fn posix_default() -> Self {
        Self {
            lines: true,
            words: true,
            bytes: true,
            chars: false,
        }
    }

    /// Builds a filter from JSON-RPC params; with no `true` flags, includes all metrics.
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

    /// Builds a filter from CLI `-l`/`-w`/`-c`/`-m`; with no flags, uses [`Self::posix_default`].
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

    /// Returns how many metric columns are active.
    pub fn active_columns(&self) -> usize {
        usize::from(self.lines)
            + usize::from(self.words)
            + usize::from(self.bytes)
            + usize::from(self.chars)
    }

    /// Returns active column labels in display order.
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

    /// Formats counts from [`WcStats`] for the active columns.
    pub fn format_counts(&self, stats: &WcStats) -> Vec<String> {
        self.format_optional_counts(
            Some(stats.lines()),
            Some(stats.words()),
            Some(stats.bytes()),
            Some(stats.chars()),
        )
    }

    /// Formats optional counts for the active columns (`None` → `"0"`).
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

/// Builds a JSON object of filtered metrics from [`WcStats`].
pub fn wc_stats_to_value(stats: &WcStats, filter: &WcMetricsFilter) -> Value {
    Value::Object(wc_metrics_object(stats, filter))
}

/// Builds a JSON file row (`path` plus filtered metrics).
pub fn wc_file_to_value(path: &str, stats: &WcStats, filter: &WcMetricsFilter) -> Value {
    let mut m = wc_metrics_object(stats, filter);
    m.insert("path".into(), json!(path));
    Value::Object(m)
}

/// Sums metric totals across rows into one [`WcStats`].
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

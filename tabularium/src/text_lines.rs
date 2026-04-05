//! Logical line slicing (`\\n`-separated) for documents.

/// How many logical lines to show from the end of a document, or from a 1-based start line (GNU `tail -n +N`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TailMode {
    /// Last `n` logical lines; `n == 0` yields no lines.
    Last(u32),
    /// GNU `tail -n +start`: lines `start..=EOF` (1-based); `start >= 1`.
    FromLine(u32),
}

impl TailMode {
    /// Parse external wire/JSON string form: must be `"+N"` with `N >= 1`.
    pub fn from_plus_wire_str(s: &str) -> Result<Self, String> {
        let s = s.trim();
        let rest = s
            .strip_prefix('+')
            .ok_or_else(|| "lines string must be \"+N\" (start at logical line N)".to_string())?;
        Self::digits_after_plus(rest)
    }

    /// CLI `-n` token: decimal = last N lines; `+K` = from line K (1-based), `K >= 1`.
    pub fn parse_cli_token(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty tail line count".into());
        }
        if let Some(rest) = s.strip_prefix('+') {
            Self::digits_after_plus(rest)
        } else {
            let n: u32 = s.parse().map_err(|_| format!("invalid tail -n: {s:?}"))?;
            Ok(TailMode::Last(n))
        }
    }

    fn digits_after_plus(rest: &str) -> Result<Self, String> {
        if rest.is_empty() {
            return Err("invalid lines: bare \"+\"".into());
        }
        if !rest.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("invalid \"+N\" form: \"+{rest}\""));
        }
        let n: u32 = rest
            .parse()
            .map_err(|_| format!("invalid \"+N\" number: \"+{rest}\""))?;
        if n == 0 {
            return Err("+0 is not allowed".into());
        }
        Ok(TailMode::FromLine(n))
    }
}

/// First `lines` logical lines; `lines == 0` yields no lines.
pub fn head_logical_lines(content: &str, lines: u32) -> String {
    if lines == 0 {
        return String::new();
    }
    let n = lines as usize;
    let v: Vec<&str> = content.lines().collect();
    let end = n.min(v.len());
    v[..end].join("\n")
}

/// Last `lines` lines; `lines == 0` yields no lines.
pub fn tail_logical_lines(content: &str, lines: u32) -> String {
    apply_tail_logical_lines(content, TailMode::Last(lines))
}

/// Apply [`TailMode`] to `content` (logical `\n`-separated lines).
pub fn apply_tail_logical_lines(content: &str, mode: TailMode) -> String {
    match mode {
        TailMode::Last(n) => {
            if n == 0 {
                return String::new();
            }
            let n = n as usize;
            let v: Vec<&str> = content.lines().collect();
            let start = v.len().saturating_sub(n);
            v[start..].join("\n")
        }
        TailMode::FromLine(start_line) => {
            let v: Vec<&str> = content.lines().collect();
            let idx = (start_line as usize).saturating_sub(1);
            if idx >= v.len() {
                String::new()
            } else {
                v[idx..].join("\n")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_last_and_from_line() {
        let s = "a\nb\nc";
        assert_eq!(head_logical_lines(s, 0), "");
        assert_eq!(tail_logical_lines(s, 0), "");
        assert_eq!(apply_tail_logical_lines(s, TailMode::Last(0)), "");
        assert_eq!(apply_tail_logical_lines(s, TailMode::Last(2)), "b\nc");
        assert_eq!(apply_tail_logical_lines(s, TailMode::FromLine(1)), s);
        assert_eq!(apply_tail_logical_lines(s, TailMode::FromLine(2)), "b\nc");
        assert_eq!(apply_tail_logical_lines(s, TailMode::FromLine(3)), "c");
        assert_eq!(apply_tail_logical_lines(s, TailMode::FromLine(4)), "");
    }

    #[test]
    fn plus_wire_accepts_and_rejects() {
        assert_eq!(
            TailMode::from_plus_wire_str("+1").unwrap(),
            TailMode::FromLine(1)
        );
        assert!(TailMode::from_plus_wire_str("+0").is_err());
        assert!(TailMode::from_plus_wire_str("1").is_err());
        assert!(TailMode::from_plus_wire_str("+x").is_err());
        assert!(TailMode::from_plus_wire_str("").is_err());
    }

    #[test]
    fn cli_token() {
        assert_eq!(TailMode::parse_cli_token("10").unwrap(), TailMode::Last(10));
        assert_eq!(
            TailMode::parse_cli_token("+2").unwrap(),
            TailMode::FromLine(2)
        );
        assert!(TailMode::parse_cli_token("+0").is_err());
    }
}

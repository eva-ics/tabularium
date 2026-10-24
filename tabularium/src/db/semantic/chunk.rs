//! UTF-8 byte windows with exact source offsets; no generated quotations.

#[derive(Debug, Clone)]
pub(super) struct TextChunk {
    pub start: usize,
    pub end: usize,
    pub line_start: usize,
    pub line_end: usize,
}

pub(super) fn split(body: &str, size: usize, overlap: usize) -> Vec<TextChunk> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < body.len() {
        let mut end = (start + size).min(body.len());
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        if end < body.len() {
            // Search on character boundaries, including for non-ASCII documents.
            let half = start + (end - start) / 2;
            let half = (half..=end)
                .find(|&i| body.is_char_boundary(i))
                .unwrap_or(end);
            if let Some(i) = body[half..end].rfind('\n') {
                end = half + i + 1;
            }
        }
        if end <= start {
            break;
        }
        if !body[start..end].trim().is_empty() {
            let line_start = 1 + body[..start].bytes().filter(|&b| b == b'\n').count();
            let line_end = line_start
                + body[start..end]
                    .trim_end_matches('\n')
                    .bytes()
                    .filter(|&b| b == b'\n')
                    .count();
            out.push(TextChunk {
                start,
                end,
                line_start,
                line_end,
            });
        }
        if end == body.len() {
            break;
        }
        let mut next = end.saturating_sub(overlap).max(start + 1);
        while next < end && !body.is_char_boundary(next) {
            next += 1;
        }
        start = next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_offsets_cover_source_and_lines() {
        let s = "перший рядок\nдругий рядок\n".repeat(20);
        let chunks = split(&s, 37, 6);
        assert_eq!(chunks.first().unwrap().start, 0);
        assert_eq!(chunks.last().unwrap().end, s.len());
        for c in &chunks {
            assert!(s.get(c.start..c.end).is_some());
            assert!(c.end - c.start <= 37);
            assert_eq!(
                c.line_start,
                1 + s[..c.start].bytes().filter(|&b| b == b'\n').count()
            );
        }
        for w in chunks.windows(2) {
            assert!(w[1].start <= w[0].end);
            assert!(w[1].start > w[0].start);
        }
    }
}

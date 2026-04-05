//! Markdown transcript for chat TUI: termimad parse + wrap, then ratatui `Text` (no ANSI).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use termimad::crossterm::style::{Attribute, Color as CrosstermColor, ContentStyle};
use termimad::minimad::Compound;
use termimad::{CompositeKind, FmtComposite, FmtLine, FmtTableRow, FmtText, MadSkin};

#[allow(clippy::match_same_arms)] // several crossterm "dark" hues map to ratatui DarkGray
fn crossterm_to_ratatui_color(c: CrosstermColor) -> Color {
    match c {
        CrosstermColor::Reset => Color::Reset,
        CrosstermColor::Black => Color::Black,
        CrosstermColor::DarkGrey => Color::DarkGray,
        CrosstermColor::Red => Color::Red,
        CrosstermColor::DarkRed => Color::DarkGray,
        CrosstermColor::Green => Color::Green,
        CrosstermColor::DarkGreen => Color::DarkGray,
        CrosstermColor::Yellow => Color::Yellow,
        CrosstermColor::DarkYellow => Color::DarkGray,
        CrosstermColor::Blue => Color::Blue,
        CrosstermColor::DarkBlue => Color::DarkGray,
        CrosstermColor::Magenta => Color::Magenta,
        CrosstermColor::DarkMagenta => Color::DarkGray,
        CrosstermColor::Cyan => Color::Cyan,
        CrosstermColor::DarkCyan => Color::DarkGray,
        CrosstermColor::White => Color::White,
        CrosstermColor::Grey => Color::Gray,
        CrosstermColor::Rgb { r, g, b } => Color::Rgb(r, g, b),
        CrosstermColor::AnsiValue(v) => Color::Indexed(v),
    }
}

fn content_style_to_ratatui(cs: &ContentStyle) -> Style {
    let mut s = Style::default();
    if let Some(fg) = cs.foreground_color
        && !matches!(fg, CrosstermColor::Reset)
    {
        s = s.fg(crossterm_to_ratatui_color(fg));
    }
    if let Some(bg) = cs.background_color
        && !matches!(bg, CrosstermColor::Reset)
    {
        s = s.bg(crossterm_to_ratatui_color(bg));
    }
    let a = cs.attributes;
    if a.has(Attribute::Bold) {
        s = s.add_modifier(Modifier::BOLD);
    }
    if a.has(Attribute::Italic) {
        s = s.add_modifier(Modifier::ITALIC);
    }
    if a.has(Attribute::Underlined) {
        s = s.add_modifier(Modifier::UNDERLINED);
    }
    if a.has(Attribute::CrossedOut) {
        s = s.add_modifier(Modifier::CROSSED_OUT);
    }
    if a.has(Attribute::Dim) {
        s = s.add_modifier(Modifier::DIM);
    }
    if a.has(Attribute::Reverse) {
        s = s.add_modifier(Modifier::REVERSED);
    }
    s
}

fn compound_style_to_ratatui(cs: &termimad::CompoundStyle) -> Style {
    content_style_to_ratatui(&cs.object_style)
}

fn push_compound_spans(
    skin: &MadSkin,
    line_style: &termimad::LineStyle,
    compounds: &[Compound<'_>],
    spans: &mut Vec<Span<'static>>,
) {
    for compound in compounds {
        if compound.src.is_empty() {
            continue;
        }
        let cs = skin.compound_style(line_style, compound);
        let style = compound_style_to_ratatui(&cs);
        spans.push(Span::styled(compound.src.to_string(), style));
    }
}

fn fmt_composite_to_line(skin: &MadSkin, fc: &FmtComposite<'_>) -> Line<'static> {
    let line_style = skin.line_style(fc.kind);
    let mut spans: Vec<Span<'static>> = Vec::new();

    match fc.kind {
        CompositeKind::ListItem(level) => {
            let indent = (level as usize).saturating_sub(1) * 2;
            spans.push(Span::raw(" ".repeat(indent)));
            spans.push(Span::styled("• ", Style::default().fg(Color::DarkGray)));
        }
        CompositeKind::ListItemFollowUp(level) => {
            spans.push(Span::raw(" ".repeat((level as usize) * 2)));
        }
        CompositeKind::Quote => {
            spans.push(Span::styled("▐ ", Style::default().fg(Color::DarkGray)));
        }
        _ => {}
    }

    push_compound_spans(skin, line_style, &fc.compounds, &mut spans);

    if spans.is_empty() {
        Line::from("")
    } else {
        Line::from(spans)
    }
}

fn table_row_to_line(skin: &MadSkin, row: &FmtTableRow<'_>) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, cell) in row.cells.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
        }
        let ls = skin.line_style(cell.kind);
        push_compound_spans(skin, ls, &cell.compounds, &mut spans);
    }
    if spans.is_empty() {
        Line::from("")
    } else {
        Line::from(spans)
    }
}

fn fmt_lines_to_ratatui(skin: &MadSkin, fmt: &FmtText<'_, '_>) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for line in &fmt.lines {
        match line {
            FmtLine::Normal(fc) => out.push(fmt_composite_to_line(skin, fc)),
            FmtLine::TableRow(row) => out.push(table_row_to_line(skin, row)),
            FmtLine::TableRule(_) | FmtLine::HorizontalRule => out.push(Line::from(Span::styled(
                "─".repeat(16),
                Style::default().fg(Color::DarkGray),
            ))),
        }
    }
    if out.is_empty() {
        out.push(Line::from(""));
    }
    out
}

/// Wrapped markdown as ratatui lines (styled, no escape sequences).
pub(crate) fn markdown_transcript_text(
    transcript: &str,
    width: usize,
    skin: &MadSkin,
) -> Text<'static> {
    let width = width.max(3);
    if transcript.is_empty() {
        return Text::from(vec![Line::from("")]);
    }
    let fmt = FmtText::from(skin, transcript, Some(width));
    let lines = fmt_lines_to_ratatui(skin, &fmt);
    Text::from(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bold_emits_separate_styled_span() {
        let skin = MadSkin::default();
        let t = markdown_transcript_text("plain **bold** tail", 80, &skin);
        let flat: String = t
            .lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert_eq!(flat, "plain bold tail");
        let has_bold = t.lines.iter().any(|l| {
            l.spans
                .iter()
                .any(|s| s.style.add_modifier.contains(Modifier::BOLD))
        });
        assert!(has_bold, "expected a bold span");
    }

    #[test]
    fn header_line_has_style() {
        let skin = MadSkin::default();
        let t = markdown_transcript_text("# Title here", 40, &skin);
        assert!(!t.lines.is_empty());
        let joined: String = t
            .lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(joined.contains("Title"));
    }
}

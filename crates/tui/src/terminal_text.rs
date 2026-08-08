//! Terminal-safe text normalization for all dynamic TUI content.
//!
//! Ratatui's diff buffer assumes every `Span` contains printable glyphs.
//! Passing terminal controls through `Span::raw` lets the terminal move its
//! real cursor without ratatui knowing, permanently desynchronizing the two
//! grids. Normalize once when text enters the UI model; the frame hot path
//! remains allocation- and scan-free.

use std::borrow::Cow;

use ratatui::text::Line;

const TAB_REPLACEMENT: &str = "    ";

#[derive(Clone, Copy)]
enum Layout {
    Multiline,
    SingleLine,
}

fn is_terminal_control(ch: char) -> bool {
    let cp = ch as u32;
    cp <= 0x1f || cp == 0x7f || (0x80..=0x9f).contains(&cp)
}

fn needs_normalization(input: &str, layout: Layout) -> bool {
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        let c0 = byte < 0x20 && !(byte == b'\n' && matches!(layout, Layout::Multiline));
        let c1 = byte == 0xc2
            && bytes
                .get(i + 1)
                .is_some_and(|next| (0x80..=0x9f).contains(next));
        if c0 || byte == 0x7f || c1 {
            return true;
        }
        i += 1;
    }
    false
}

fn normalize(input: &str, layout: Layout) -> Cow<'_, str> {
    if !needs_normalization(input, layout) {
        return Cow::Borrowed(input);
    }

    let mut output = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\n' if matches!(layout, Layout::Multiline) => output.push('\n'),
            '\n' if matches!(layout, Layout::SingleLine) => output.push(' '),
            '\t' => output.push_str(TAB_REPLACEMENT),
            '\r' => {}
            _ if is_terminal_control(ch) => {}
            _ => output.push(ch),
        }
    }
    Cow::Owned(output)
}

/// Normalize text that may contain real line breaks. Safe input is returned
/// borrowed, so normal streaming deltas incur no allocation.
pub(crate) fn sanitize_multiline(input: &str) -> Cow<'_, str> {
    normalize(input, Layout::Multiline)
}

/// Normalize metadata that must remain on one terminal row.
pub(crate) fn sanitize_single_line(input: &str) -> Cow<'_, str> {
    normalize(input, Layout::SingleLine)
}

/// Sanitize a styled line without changing its styles. Safe spans retain
/// their existing `Cow`, avoiding replacement allocations.
pub(crate) fn sanitize_line(mut line: Line<'static>) -> Line<'static> {
    for span in &mut line.spans {
        if let Cow::Owned(clean) = sanitize_single_line(span.content.as_ref()) {
            span.content = Cow::Owned(clean);
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        style::{Color, Style},
        text::Span,
    };

    #[test]
    fn safe_text_is_borrowed_without_allocation() {
        assert!(matches!(
            sanitize_multiline("plain 中文 emoji 💭\nnext"),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            sanitize_single_line("plain 中文"),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn multiline_removes_terminal_controls_and_expands_tabs() {
        let input = "old\rNEW\x08\x1b[2J\u{009b}31m\x7f\tend\nnext";
        assert_eq!(sanitize_multiline(input), "oldNEW[2J31m    end\nnext");
    }

    #[test]
    fn single_line_flattens_newlines_and_preserves_printable_unicode() {
        assert_eq!(
            sanitize_single_line("a\r\nb\tc\u{0007}你好💭"),
            "a b    c你好💭"
        );
    }

    #[test]
    fn styled_line_keeps_style_while_sanitizing_content() {
        let style = Style::default().fg(Color::Cyan);
        let line = Line::from(vec![Span::styled("safe", style), Span::raw("bad\r\x1b")]);
        let clean = sanitize_line(line);

        assert_eq!(clean.spans[0].content, "safe");
        assert_eq!(clean.spans[0].style, style);
        assert_eq!(clean.spans[1].content, "bad");
    }
}

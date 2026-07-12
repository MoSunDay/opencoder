//! Mouse-driven text selection in the chat body + OSC52 clipboard copy.
//!
//! The body renders `chat.flatten()` wrapped at `text_w` columns. Selection is
//! tracked in *absolute content rows* (screen row + scroll offset) so it stays
//! anchored to the text while the viewport scrolls. A drag selects whole
//! logical lines (a logical line may wrap across several screen rows); on
//! mouse-up the selected text is copied to the system clipboard via an OSC52
//! escape sequence (works over SSH; no external dependency).
//!
//! Scope (v1): line-range selection. The selection is cleared once copied.

use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::chat::ChatView;

/// An active selection: an absolute content-row range `[a, b]` (inclusive,
/// un-normalised — either end may be the anchor or the current drag position).
/// `None` means no active selection. Absolute rows are `screen_row + scroll`.
pub type SelRange = (u16, u16);

/// Normalise a selection to `(lo, hi)` inclusive.
pub fn sel_range(s: SelRange) -> (u16, u16) {
    (s.0.min(s.1), s.0.max(s.1))
}

/// Map a screen `row` (terminal coordinate) to the absolute content row it
/// covers within the body's inner text area, accounting for `scroll`. Returns
/// `None` when the row is outside the text area (borders / outside the body).
/// Inner area = body rect minus its 1-cell border on every side.
pub fn abs_row_at(body: Rect, row: u16, scroll: u16) -> Option<u16> {
    let inner_y = body.y.saturating_add(1);
    let inner_h = body.height.saturating_sub(2);
    if row >= inner_y && row < inner_y.saturating_add(inner_h) {
        Some(row.saturating_sub(inner_y).saturating_add(scroll))
    } else {
        None
    }
}

/// On mouse-up: extract the selected text from the *viewed* chat and copy it to
/// the clipboard. A bare click (anchor == active) copies nothing. `body` is the
/// body's outer rect (used to derive the wrap width); pass `None` if unknown.
pub fn finish_copy(viewed: &ChatView, body: Option<Rect>, sel: SelRange) {
    let (lo, hi) = sel_range(sel);
    if lo == hi {
        return; // bare click — no drag, no copy
    }
    let text_w = body.map(|r| r.width.saturating_sub(3)).unwrap_or(0);
    let text = extract_text(viewed, text_w, sel);
    if !text.trim().is_empty() {
        copy_osc52(&text);
    }
}

/// Number of screen rows a wrapped logical line occupies at width `w`,
/// matching ratatui's `Paragraph` wrapping exactly. An empty line is 1 row.
fn wrapped_rows(line: &Line<'_>, w: u16) -> u32 {
    Paragraph::new(line.clone())
        .wrap(Wrap { trim: false })
        .line_count(w) as u32
}

/// Extract the text of every logical line whose wrapped screen-row span
/// intersects the absolute row range `[lo, hi]`. Lines are joined with `\n`.
/// Whole logical lines are taken even for partial row coverage — this is the
/// "line-range" selection model (v1).
pub fn extract_text(chat: &ChatView, text_w: u16, sel: SelRange) -> String {
    let (lo, hi) = sel_range(sel);
    if text_w == 0 {
        return String::new();
    }
    let lines = chat.flatten();
    let mut row: u32 = 0;
    let mut out: Vec<String> = Vec::new();
    for line in &lines {
        let h = wrapped_rows(line, text_w);
        let span_lo = row;
        let span_hi = row.saturating_add(h);
        // Intersection of [span_lo, span_hi) with [lo, hi].
        if span_hi > lo as u32 && span_lo <= hi as u32 {
            let s: String = line.spans.iter().map(|sp| sp.content.as_ref()).collect();
            out.push(s);
        }
        row = span_hi;
        if span_lo > hi as u32 {
            break;
        }
    }
    out.join("\n")
}

/// Overlay inverse-video highlight on the selected rows currently visible in
/// `text_area`. `scroll_y` is the body's scroll offset; `sel` is the absolute
/// content-row range. Rows outside the viewport are clipped. Drawn after the
/// paragraph so the highlight sits on top of the text.
pub fn render_overlay(f: &mut Frame, text_area: Rect, scroll_y: u16, sel: Option<SelRange>) {
    let (lo, hi) = match sel.map(sel_range) {
        Some(r) => r,
        None => return,
    };
    if text_area.height == 0 {
        return;
    }
    let view_top = scroll_y;
    let view_bot = scroll_y.saturating_add(text_area.height);
    let s_lo = lo.max(view_top);
    // `view_bot` is exclusive; the last visible absolute row is `view_bot - 1`.
    let s_hi = hi.min(view_bot.saturating_sub(1));
    if s_hi < s_lo {
        return;
    }
    let buf = f.buffer_mut();
    let first = s_lo.saturating_sub(scroll_y);
    let last = s_hi.saturating_sub(scroll_y);
    for r in first..=last {
        let y = text_area.y + r;
        if y >= text_area.bottom() {
            break;
        }
        for x in text_area.x..text_area.right() {
            let cell = &mut buf[(x, y)];
            // Inverse video — the canonical selection look. Read the current
            // style, then swap fg/bg via set_style (ratatui's Cell exposes
            // style()/set_style rather than fg()/bg() accessors).
            let cur = cell.style();
            let inv_fg = cur.bg.unwrap_or(ratatui::style::Color::Reset);
            let inv_bg = cur.fg.unwrap_or(ratatui::style::Color::Reset);
            cell.set_style(ratatui::style::Style::default().fg(inv_fg).bg(inv_bg));
        }
    }
}

/// Copy `text` to the system clipboard via OSC 52 (terminal clipboard escape).
/// Works over SSH and in most modern terminals (xterm/tmux/kitty/alacritty,
/// iTerm2, Windows Terminal). Best-effort: a failed write is swallowed — a
/// clipboard error must never crash the UI.
pub fn copy_osc52(text: &str) {
    let payload = base64_encode(text.as_bytes());
    // ESC ] 52 ; <clipboard=c> ; <base64> BEL
    let mut seq = String::with_capacity(payload.len() + 16);
    seq.push_str("\u{1b}]52;c;");
    seq.push_str(&payload);
    seq.push('\u{07}');
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = out.write_all(seq.as_bytes());
    let _ = out.flush();
}

/// Minimal RFC-4648 base64 encoder (standard alphabet, `=` padding). Vendored
/// to avoid pulling in a dependency for the one place clipboard copy needs it.
fn base64_encode(input: &[u8]) -> String {
    const T: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let block = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[((block >> 18) & 0x3f) as usize] as char);
        out.push(T[((block >> 12) & 0x3f) as usize] as char);
        if chunk.len() >= 2 {
            out.push(T[((block >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() == 3 {
            out.push(T[(block & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Line;

    /// Build a view whose flattened lines are exactly `lines` (one Marker block
    /// per line). Markers render verbatim, so the test is independent of the
    /// Assistant markdown renderer (which prepends a `say:` header + indent).
    fn view_from_lines(lines: &[&str]) -> ChatView {
        let mut v = ChatView::default();
        for &l in lines {
            v.push_marker(Line::from(l.to_string()));
        }
        v
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        // UTF-8 bytes are encoded verbatim.
        assert_eq!(base64_encode("中".as_bytes()), "5Lit");
    }

    #[test]
    fn sel_range_normalises_either_direction() {
        assert_eq!(sel_range((5, 2)), (2, 5));
        assert_eq!(sel_range((2, 5)), (2, 5));
        assert_eq!(sel_range((3, 3)), (3, 3));
    }

    #[test]
    fn abs_row_maps_screen_to_content_with_scroll() {
        // Body at y=10, height=12 → inner text area y=11..21 (10 rows).
        let body = Rect::new(0, 10, 80, 12);
        // Top inner row, no scroll → content row 0.
        assert_eq!(abs_row_at(body, 11, 0), Some(0));
        // 5 rows down, scroll=20 → content row 25.
        assert_eq!(abs_row_at(body, 16, 20), Some(25));
        // On the top border (y=10) → None.
        assert_eq!(abs_row_at(body, 10, 0), None);
        // Below the inner area (y=21 is the bottom border) → None.
        assert_eq!(abs_row_at(body, 21, 0), None);
    }

    #[test]
    fn extract_single_visible_line() {
        // One marker line "hello" at absolute row 0; select row 0.
        let v = view_from_lines(&["hello"]);
        assert_eq!(extract_text(&v, 40, (0, 0)), "hello");
    }

    #[test]
    fn extract_range_across_lines() {
        let v = view_from_lines(&["aaa", "bbb", "ccc"]);
        // Wide enough that each logical line is exactly one screen row.
        assert_eq!(extract_text(&v, 80, (0, 1)), "aaa\nbbb");
        // Single middle line.
        assert_eq!(extract_text(&v, 80, (1, 1)), "bbb");
        // Full range.
        assert_eq!(extract_text(&v, 80, (0, 2)), "aaa\nbbb\nccc");
    }

    #[test]
    fn extract_whole_logical_line_when_partially_covered() {
        // A long line wrapping across multiple screen rows at narrow width.
        let long = "abcdefghijklmnop"; // 16 chars
        let v = view_from_lines(&[long]);
        // At width 4 it wraps to several screen rows. Selecting only the
        // second screen row (row 1) still yields the entire logical line.
        let w = 4u16;
        let rows = wrapped_rows(&v.flatten()[0], w);
        assert!(rows >= 2, "expected wrapping, got {rows} rows");
        assert_eq!(extract_text(&v, w, (1, 1)), long);
    }

    #[test]
    fn extract_empty_when_text_w_zero() {
        let v = view_from_lines(&["hello"]);
        assert_eq!(extract_text(&v, 0, (0, 0)), "");
    }

    #[test]
    fn osc52_sequence_format() {
        // "hi" -> base64 "aGk="; the encoder backs the payload embedded in the
        // OSC52 framing. We can't intercept stdout here, but we assert the
        // encoder and that copy_osc52 must not panic on arbitrary unicode.
        assert_eq!(base64_encode(b"hi"), "aGk=");
        copy_osc52("hello 世界 \u{1f600}");
    }
}

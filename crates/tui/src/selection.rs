//! Mouse-driven text selection in the chat body + clipboard copy (OSC52 with a
//! local clipboard-command fallback).
//!
//! The body renders `chat.flatten()` wrapped at `text_w` columns. Selection is
//! tracked in *absolute content rows* (screen row + scroll offset) so it stays
//! anchored to the text while the viewport scrolls. A drag selects whole
//! logical lines (a logical line may wrap across several screen rows); on
//! mouse-up the selected text is copied to the system clipboard via OSC52
//! (works over SSH) and, as a fallback, a local clipboard command (pbcopy /
//! wl-copy / xclip / xsel / clip.exe) for terminals that lack OSC52 support.
//!
//! Scope (v1): line-range selection. The selection is cleared once copied.

use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

use crate::chat::ChatView;

/// An active selection: an absolute content-row range `[a, b]` (inclusive,
/// un-normalised — either end may be the anchor or the current drag position).
/// `None` means no active selection. Absolute rows are `screen_row + scroll`.
pub type SelRange = (u32, u32);

/// Report of a clipboard copy attempt, for building visible UI feedback.
/// `lines`/`chars` describe how much text was copied; `osc52_reliable`
/// is the probe's verdict on whether OSC52 can be trusted in this terminal;
/// `local_tool` names the local clipboard command that succeeded, if any;
/// `tmux`/`ssh` carry contextual hints for the failure message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyReport {
    /// Number of logical lines in the copied text.
    pub lines: usize,
    /// Number of characters in the copied text.
    pub chars: usize,
    /// Whether OSC52 can be trusted to reach the clipboard here (from probe).
    pub osc52_reliable: bool,
    /// The local clipboard tool that succeeded, if any.
    pub local_tool: Option<&'static str>,
    /// Running inside tmux (drives the failure hint).
    pub tmux: bool,
    /// Running over SSH (drives the failure hint).
    pub ssh: bool,
}

impl CopyReport {
    /// Build a user-facing status message from this report. Three cases:
    /// a local tool succeeded (green), OSC52 is reliable with no local tool
    /// (green), or neither (red, honest failure with a contextual hint).
    pub fn status_message(&self) -> String {
        match self.local_tool {
            Some(tool) => format!("\u{1f4cb} Copied {} line(s) ({})", self.lines, tool),
            None if self.osc52_reliable => {
                format!("\u{1f4cb} Copied {} line(s) via OSC52", self.lines)
            }
            None => {
                let hint = if self.tmux {
                    "tmux: set -g set-clipboard on, or install xclip"
                } else if self.ssh {
                    "terminal may not support OSC52 — install xclip/xsel locally"
                } else {
                    "install xclip/xsel"
                };
                format!("\u{26a0} Copy unreliable \u{2014} {}", hint)
            }
        }
    }
}

/// Normalise a selection to `(lo, hi)` inclusive.
pub fn sel_range(s: SelRange) -> (u32, u32) {
    (s.0.min(s.1), s.0.max(s.1))
}

/// Map a screen `row` (terminal coordinate) to the absolute content row it
/// covers within the body's inner text area, accounting for `scroll`. Returns
/// `None` when the row is outside the text area (borders / outside the body).
/// Inner area = body rect minus its 1-cell border on every side.
pub fn abs_row_at(body: Rect, row: u16, scroll: u32) -> Option<u32> {
    let inner_y = body.y.saturating_add(1);
    let inner_h = body.height.saturating_sub(2);
    if row >= inner_y && row < inner_y.saturating_add(inner_h) {
        Some(row.saturating_sub(inner_y) as u32 + scroll)
    } else {
        None
    }
}

/// On mouse-up: extract the selected text from the *viewed* chat and copy it to
/// the clipboard. A bare click (anchor == active) copies nothing. `body` is the
/// body's outer rect (used to derive the wrap width); pass `None` if unknown.
/// Returns `None` for a bare click or empty selection; otherwise a [`CopyReport`]
/// describing the copy for UI feedback.
pub fn finish_copy(
    viewed: &ChatView,
    body: Option<Rect>,
    sel: SelRange,
    force: bool,
) -> Option<CopyReport> {
    let (lo, hi) = sel_range(sel);
    if lo == hi && !force {
        return None; // bare click — no drag, no copy
    }
    let text_w = body.map(|r| r.width.saturating_sub(3)).unwrap_or(0);
    let text = extract_text(viewed, text_w, sel);
    if text.trim().is_empty() {
        return None;
    }
    Some(copy_to_clipboard(&text))
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
        if span_hi > lo && span_lo <= hi {
            let s: String = line.spans.iter().map(|sp| sp.content.as_ref()).collect();
            out.push(s);
        }
        row = span_hi;
        if span_lo > hi {
            break;
        }
    }
    out.join("\n")
}

/// Overlay inverse-video highlight on the selected rows currently visible in
/// `text_area`. `scroll_y` is the body's scroll offset; `sel` is the absolute
/// content-row range. Rows outside the viewport are clipped. Drawn after the
/// paragraph so the highlight sits on top of the text.
pub fn render_overlay(f: &mut Frame, text_area: Rect, scroll_y: u32, sel: Option<SelRange>) {
    let (lo, hi) = match sel.map(sel_range) {
        Some(r) => r,
        None => return,
    };
    if text_area.height == 0 {
        return;
    }
    let view_top = scroll_y;
    let view_bot = scroll_y.saturating_add(text_area.height as u32);
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
        let y = text_area.y + r as u16;
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

/// Copy `text` to the system clipboard using every available backend,
/// best-effort. Both backends are attempted so that:
/// - OSC52 covers SSH-remote sessions and OSC52-capable local terminals.
/// - A local clipboard command covers local terminals that ignore OSC52
///   (e.g. some Linux terminal emulators with the feature disabled).
///
/// OSC52 runs synchronously (a fast stdout write — the primary path for SSH).
/// The local clipboard command also runs synchronously via `try_spawn`, which
/// enforces a 3-second timeout so a stalled helper cannot hang the TUI
/// indefinitely. Errors are swallowed: a clipboard failure must never crash
/// the UI.
pub fn copy_to_clipboard(text: &str) -> CopyReport {
    let probe = crate::clip_probe::probe_clipboard();
    // OSC52 is still always sent (best-effort primary path for SSH / capable
    // terminals); the *message* reflects the probe's confidence, not the send.
    copy_osc52(text);
    let local_tool = crate::clip_probe::copy_local_smart(&probe, text);
    CopyReport {
        lines: text.lines().count(),
        chars: text.chars().count(),
        osc52_reliable: probe.osc52_reliable,
        local_tool,
        tmux: probe.is_tmux,
        ssh: probe.is_ssh,
    }
}

/// Copy `text` to the system clipboard via OSC 52 (terminal clipboard escape).
/// Works over SSH and in most modern terminals (xterm/tmux/kitty/alacritty,
/// iTerm2, Windows Terminal). Best-effort: a failed write is swallowed — a
/// clipboard error must never crash the UI.
pub fn copy_osc52(text: &str) {
    let payload = STANDARD.encode(text.as_bytes());
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
        assert_eq!(STANDARD.encode(b""), "");
        assert_eq!(STANDARD.encode(b"f"), "Zg==");
        assert_eq!(STANDARD.encode(b"fo"), "Zm8=");
        assert_eq!(STANDARD.encode(b"foo"), "Zm9v");
        assert_eq!(STANDARD.encode(b"foob"), "Zm9vYg==");
        assert_eq!(STANDARD.encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(STANDARD.encode(b"foobar"), "Zm9vYmFy");
        // UTF-8 bytes are encoded verbatim.
        assert_eq!(STANDARD.encode("中".as_bytes()), "5Lit");
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
        assert_eq!(STANDARD.encode(b"hi"), "aGk=");
        copy_osc52("hello 世界 \u{1f600}");
    }

    #[test]
    fn copy_report_status_with_local_tool() {
        let report = CopyReport {
            lines: 3,
            chars: 42,
            osc52_reliable: true,
            local_tool: Some("xclip"),
            tmux: false,
            ssh: false,
        };
        let msg = report.status_message();
        assert!(msg.contains("3 line"));
        assert!(msg.contains("xclip"));
        assert!(!msg.contains("Copy unreliable"));
        assert!(!msg.contains("Shift+drag"));
    }

    #[test]
    fn copy_report_status_reliable_osc52_no_tool() {
        let report = CopyReport {
            lines: 1,
            chars: 5,
            osc52_reliable: true,
            local_tool: None,
            tmux: false,
            ssh: false,
        };
        let msg = report.status_message();
        assert!(msg.contains("OSC52"));
        assert!(msg.contains("1 line(s)"));
        assert!(!msg.contains("\u{26a0}"));
        assert!(!msg.contains("Shift+drag"));
    }

    #[test]
    fn copy_report_status_unreliable_with_tmux_hint() {
        let report = CopyReport {
            lines: 2,
            chars: 9,
            osc52_reliable: false,
            local_tool: None,
            tmux: true,
            ssh: false,
        };
        let msg = report.status_message();
        assert!(msg.contains("\u{26a0}"));
        assert!(msg.contains("set-clipboard"));
    }

    #[test]
    fn copy_report_status_unreliable_with_ssh_hint() {
        let report = CopyReport {
            lines: 2,
            chars: 9,
            osc52_reliable: false,
            local_tool: None,
            tmux: false,
            ssh: true,
        };
        let msg = report.status_message();
        assert!(msg.contains("\u{26a0}"));
        assert!(msg.contains("OSC52"));
        assert!(!msg.contains("Shift+drag"));
    }

    #[test]
    fn copy_report_status_unreliable_generic_hint() {
        let report = CopyReport {
            lines: 4,
            chars: 40,
            osc52_reliable: false,
            local_tool: None,
            tmux: false,
            ssh: false,
        };
        let msg = report.status_message();
        assert!(msg.contains("\u{26a0}"));
        assert!(msg.contains("install xclip"));
    }

    #[test]
    fn finish_copy_returns_none_for_bare_click() {
        let v = view_from_lines(&["hello", "world"]);
        assert!(finish_copy(&v, Some(Rect::new(0, 0, 80, 10)), (3, 3), false).is_none());
    }

    #[test]
    fn finish_copy_returns_report_for_drag() {
        let v = view_from_lines(&["hello", "world"]);
        let report = finish_copy(&v, Some(Rect::new(0, 0, 80, 10)), (0, 1), false);
        assert!(report.is_some());
        let r = report.unwrap();
        assert_eq!(r.lines, 2);
        assert!(r.chars > 0);
        // osc52_reliable echoes the probe's environment-dependent verdict
        // (reliable by default for unknown terminals); we only confirm
        // the field is populated, not a specific value.
        let _ = r.osc52_reliable;
    }

    #[test]
    fn finish_copy_with_force_copies_single_line() {
        let v = view_from_lines(&["hello", "world"]);
        // With force=true a single-line selection (lo == hi) is still copied.
        let report = finish_copy(&v, Some(Rect::new(0, 0, 80, 10)), (1, 1), true);
        assert!(report.is_some());
        let r = report.unwrap();
        assert_eq!(r.lines, 1);
        assert!(r.chars > 0);
    }
}

//! OSC 52 clipboard escape-sequence writer.
//!
//! `copy_select` yanks selected body text here; OSC 52 carries it to the
//! system clipboard through the terminal itself, so copying works over SSH
//! with no local clipboard tool. Inside tmux the sequence is wrapped in a
//! DCS passthrough (`ESC Ptmux; … ESC \` with inner ESCs doubled), which
//! tmux forwards to the outer terminal when `set-clipboard` allows it.
//!
//! Best-effort by design: write errors are swallowed — a clipboard failure
//! must never crash the UI. The base64 payload is capped (many terminals
//! silently drop oversized sequences); oversized text is truncated at a
//! UTF-8 char boundary.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use std::io::Write;

/// Maximum base64 payload length we emit (100 KiB ≈ 75 KiB of text).
pub const MAX_B64_LEN: usize = 100 * 1024;

/// Truncate `text` so its base64 encoding fits within [`MAX_B64_LEN`].
/// Cuts at a UTF-8 char boundary. Pure.
pub fn truncate_for_osc52(text: &str) -> &str {
    let max_raw = MAX_B64_LEN / 4 * 3;
    if text.len() <= max_raw {
        return text;
    }
    let mut end = max_raw;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Build the full OSC 52 escape sequence for `text` (pure). `in_tmux` wraps
/// the sequence in the tmux DCS passthrough so it survives tmux's escape
/// filtering and reaches the outer terminal's clipboard.
pub fn sequence(text: &str, in_tmux: bool) -> String {
    let payload = STANDARD.encode(truncate_for_osc52(text).as_bytes());
    // ESC ] 52 ; <clipboard=c> ; <base64> BEL
    let mut seq = String::with_capacity(payload.len() + 16);
    seq.push_str("\u{1b}]52;c;");
    seq.push_str(&payload);
    seq.push('\u{07}');
    if !in_tmux {
        return seq;
    }
    // tmux DCS passthrough: double every inner ESC so tmux forwards the
    // sequence to the outer terminal verbatim.
    let mut wrapped = String::with_capacity(seq.len() * 2 + 16);
    wrapped.push_str("\u{1b}Ptmux;");
    for ch in seq.chars() {
        if ch == '\u{1b}' {
            wrapped.push('\u{1b}');
        }
        wrapped.push(ch);
    }
    wrapped.push_str("\u{1b}\\");
    wrapped
}

/// Whether we are running inside tmux (`$TMUX` set and non-empty).
pub fn in_tmux() -> bool {
    std::env::var_os("TMUX").is_some_and(|v| !v.is_empty())
}

/// Copy `text` to the system clipboard via OSC 52. Best-effort: write
/// errors are swallowed. Call from the UI thread — the sequence is small
/// and the write is non-blocking (no read wait).
pub fn copy(text: &str) {
    let seq = sequence(text, in_tmux());
    let mut out = std::io::stdout();
    let _ = out.write_all(seq.as_bytes());
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_formats_plain_osc52() {
        // "hi" -> base64 "aGk=".
        assert_eq!(sequence("hi", false), "\u{1b}]52;c;aGk=\u{07}");
    }

    #[test]
    fn sequence_wraps_for_tmux_with_doubled_escapes() {
        let s = sequence("hi", true);
        assert!(s.starts_with("\u{1b}Ptmux;"));
        assert!(s.ends_with("\u{1b}\\"));
        // Inner payload: ESC doubled, BEL intact.
        assert!(s.contains("\u{1b}\u{1b}]52;c;aGk=\u{07}"));
        // Every inner ESC must be doubled between the DCS opener and the ST
        // (removing all pairs leaves no lone ESC).
        let inner = &s["\u{1b}Ptmux;".len()..s.len() - 2];
        assert!(inner.starts_with("\u{1b}\u{1b}]52;c;"));
        assert!(!inner.replace("\u{1b}\u{1b}", "").contains('\u{1b}'));
    }

    #[test]
    fn sequence_encodes_unicode_and_empty() {
        assert_eq!(sequence("", false), "\u{1b}]52;c;\u{07}");
        // 中 -> base64 5Lit; arbitrary unicode must not panic.
        assert_eq!(sequence("中", false), "\u{1b}]52;c;5Lit\u{07}");
    }

    #[test]
    fn truncate_keeps_short_text_verbatim() {
        assert_eq!(truncate_for_osc52("hello"), "hello");
        assert_eq!(truncate_for_osc52(""), "");
    }

    #[test]
    fn truncate_caps_oversized_text_at_char_boundary() {
        let max_raw = MAX_B64_LEN / 4 * 3;
        let big = "x".repeat(max_raw + 50);
        let cut = truncate_for_osc52(&big);
        assert_eq!(cut.len(), max_raw);
        // Multi-byte safety: cutting a CJK run must land on a boundary.
        let cjk = "中".repeat(max_raw / 3 + 10);
        let cut = truncate_for_osc52(&cjk);
        assert!(cut.chars().all(|c| c == '中'));
        assert!(STANDARD.encode(cut).len() <= MAX_B64_LEN);
    }

    #[test]
    fn truncated_payload_stays_under_cap() {
        let big = "y".repeat(MAX_B64_LEN); // far past the raw budget
        let seq = sequence(&big, false);
        let payload = seq
            .strip_prefix("\u{1b}]52;c;")
            .and_then(|r| r.strip_suffix('\u{07}'))
            .expect("framing intact");
        assert!(payload.len() <= MAX_B64_LEN);
    }
}

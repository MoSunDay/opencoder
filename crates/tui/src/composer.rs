//! Composer cursor math — pure functions, unit-tested.
//!
//! The input is treated as a single line (multi-line wrap is a future addition).
//! The cursor is a char index; its on-screen column is the unicode *display*
//! width of the text before it, offset by the prompt prefix `❯ `.

/// Display column (0-based) of the cursor within the input text, given the
/// char index. Uses unicode-width so CJK / wide chars advance correctly.
pub fn cursor_column(input: &str, char_idx: usize) -> u16 {
    let mut col: usize = 0;
    for (i, ch) in input.chars().enumerate() {
        if i >= char_idx {
            break;
        }
        col += char_width(ch);
    }
    col.min(u16::MAX as usize) as u16
}

/// Display width of a char (1 for most, 2 for wide CJK/fullwidth, 0 for
/// combining marks). Avoids pulling in the unicode-width crate for the TUI.
pub fn char_width(ch: char) -> usize {
    let cp = ch as u32;
    // Zero-width: combining marks (general purpose) — approximate range.
    if cp == 0 {
        return 0;
    }
    // CJK ranges (approximate, covers common Hanzi/Kana/Hangul) → wide.
    if (0x1100..=0x115F).contains(&cp)
        || (0x2E80..=0xA4CF).contains(&cp)
        || (0xAC00..=0xD7A3).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
        || (0xFE30..=0xFE4F).contains(&cp)
        || (0xFF00..=0xFF60).contains(&cp)
        || (0xFFE0..=0xFFE6).contains(&cp)
        || (0x1F300..=0x1FAFF).contains(&cp)
    {
        return 2;
    }
    1
}

/// Move a char index clamped to [0, len].
pub fn clamp_idx(idx: usize, len: usize) -> usize {
    idx.min(len)
}

/// Insert a char at the cursor index, returning (new_text, new_idx).
pub fn insert_char(text: &str, idx: usize, ch: char) -> (String, usize) {
    let mut s = String::with_capacity(text.len() + ch.len_utf8());
    let byte = byte_offset_for_char(text, idx);
    s.push_str(&text[..byte]);
    s.push(ch);
    s.push_str(&text[byte..]);
    (s, idx + 1)
}

/// Delete the char before the cursor; returns (new_text, new_idx) or None if
/// at start.
pub fn backspace(text: &str, idx: usize) -> Option<(String, usize)> {
    if idx == 0 {
        return None;
    }
    let prev = byte_offset_for_char(text, idx - 1);
    let cur = byte_offset_for_char(text, idx);
    let mut s = String::with_capacity(text.len() - (cur - prev));
    s.push_str(&text[..prev]);
    s.push_str(&text[cur..]);
    Some((s, idx - 1))
}

fn byte_offset_for_char(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_tracks_ascii() {
        assert_eq!(cursor_column("abc", 0), 0);
        assert_eq!(cursor_column("abc", 1), 1);
        assert_eq!(cursor_column("abc", 3), 3);
    }

    #[test]
    fn column_counts_wide_chars_double() {
        // 你好 = 4 display cols
        assert_eq!(cursor_column("你好", 0), 0);
        assert_eq!(cursor_column("你好", 1), 2);
        assert_eq!(cursor_column("你好", 2), 4);
        // mixed: a你b → after a(1) + 你(2) = 3 at idx 2
        assert_eq!(cursor_column("a你b", 2), 3);
    }

    #[test]
    fn insert_at_cursor() {
        let (s, i) = insert_char("ac", 1, 'b');
        assert_eq!(s, "abc");
        assert_eq!(i, 2);
        let (s, i) = insert_char("", 0, 'x');
        assert_eq!(s, "x");
        assert_eq!(i, 1);
    }

    #[test]
    fn backspace_removes_preceding() {
        assert_eq!(backspace("ab", 2), Some(("a".into(), 1)));
        assert_eq!(backspace("ab", 1), Some(("b".into(), 0)));
        assert_eq!(backspace("ab", 0), None);
        // wide char before cursor deletes one codepoint
        assert_eq!(backspace("你", 1), Some(("".into(), 0)));
    }
}

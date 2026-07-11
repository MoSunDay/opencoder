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

/// Display width of a string: sum of per-char widths.
pub fn str_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// Truncate `s` to fit `max_w` display columns, appending an ellipsis (`…`,
/// width 1) when truncated. Returns the string unchanged if it already fits.
pub fn truncate_to_width(s: &str, max_w: usize) -> String {
    if str_width(s) <= max_w {
        return s.to_string();
    }
    let budget = max_w.saturating_sub(1);
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = char_width(ch);
        if w + cw > budget {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('\u{2026}');
    out
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

/// Compute (row, col) display position from a char index in multi-line text.
pub fn cursor_row_col(input: &str, char_idx: usize) -> (usize, usize) {
    let mut row = 0usize;
    let mut col = 0usize;
    for (i, ch) in input.chars().enumerate() {
        if i >= char_idx {
            break;
        }
        if ch == '\n' {
            row += 1;
            col = 0;
        } else {
            col += char_width(ch);
        }
    }
    (row, col)
}

/// Move cursor up/down by one visual row in multi-line text.
/// Returns the original index if already at the top/bottom row.
pub fn move_cursor_vertical(input: &str, char_idx: usize, direction: i32) -> usize {
    if input.is_empty() {
        return char_idx;
    }
    let mut line_starts: Vec<usize> = vec![0];
    for (i, ch) in input.chars().enumerate() {
        if ch == '\n' {
            line_starts.push(i + 1);
        }
    }
    let row = line_starts
        .iter()
        .rev()
        .position(|&s| s <= char_idx)
        .map(|r| line_starts.len() - 1 - r)
        .unwrap_or(0);
    let line_start = line_starts[row];
    let col: usize = input
        .chars()
        .skip(line_start)
        .take(char_idx.saturating_sub(line_start))
        .map(char_width)
        .sum();
    let target_row = row as i32 + direction;
    if target_row < 0 || target_row as usize >= line_starts.len() {
        return char_idx;
    }
    let target_row = target_row as usize;
    let target_start = line_starts[target_row];
    let target_end = if target_row + 1 < line_starts.len() {
        line_starts[target_row + 1].saturating_sub(1)
    } else {
        input.chars().count()
    };
    let mut actual = 0usize;
    let mut idx = target_start;
    for (i, ch) in input.chars().enumerate().skip(target_start) {
        if i >= target_end {
            break;
        }
        if actual + char_width(ch) > col {
            break;
        }
        actual += char_width(ch);
        idx = i + 1;
    }
    idx
}

/// Count how many display rows the input occupies at the given width.
pub fn display_rows(input: &str, width: u16) -> u16 {
    let w = (width as usize).max(1);
    let mut rows = 0usize;
    for line in input.split('\n') {
        let lw: usize = line.chars().map(char_width).sum();
        rows += if lw == 0 { 1 } else { lw.div_ceil(w) };
    }
    (rows as u16).max(1)
}

/// Insert a newline at the cursor index.
pub fn insert_newline(text: &str, idx: usize) -> (String, usize) {
    let mut s = String::with_capacity(text.len() + 1);
    let byte = byte_offset_for_char(text, idx);
    s.push_str(&text[..byte]);
    s.push('\n');
    s.push_str(&text[byte..]);
    (s, idx + 1)
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

    #[test]
    fn cursor_row_col_single_line() {
        assert_eq!(cursor_row_col("hello", 0), (0, 0));
        assert_eq!(cursor_row_col("hello", 3), (0, 3));
        assert_eq!(cursor_row_col("hello", 5), (0, 5));
    }

    #[test]
    fn cursor_row_col_multi_line() {
        let input = "abc\ndef\nghi";
        assert_eq!(cursor_row_col(input, 0), (0, 0));
        assert_eq!(cursor_row_col(input, 3), (0, 3)); // before \n
        assert_eq!(cursor_row_col(input, 4), (1, 0)); // start of line 2
        assert_eq!(cursor_row_col(input, 7), (1, 3)); // before second \n
        assert_eq!(cursor_row_col(input, 8), (2, 0)); // start of line 3
    }

    #[test]
    fn move_cursor_up_down() {
        let input = "aaaa\nbbbb\ncccc";
        // Index 2 = row 0 col 2 (display). Move down → row 1 col 2 = index 7.
        let idx = move_cursor_vertical(input, 2, 1);
        assert_eq!(cursor_row_col(input, idx), (1, 2));
        // Index 7 = row 1 col 2. Move up → row 0 col 2 = index 2.
        let idx = move_cursor_vertical(input, 7, -1);
        assert_eq!(cursor_row_col(input, idx), (0, 2));
        // Can't move up from row 0
        assert_eq!(move_cursor_vertical(input, 2, -1), 2);
        // Can't move down from last row
        assert_eq!(move_cursor_vertical(input, 10, 1), 10);
    }

    #[test]
    fn insert_newline_at_cursor() {
        let (s, i) = insert_newline("abcd", 2);
        assert_eq!(s, "ab\ncd");
        assert_eq!(i, 3);
        let (s, i) = insert_newline("", 0);
        assert_eq!(s, "\n");
        assert_eq!(i, 1);
    }

    #[test]
    fn display_rows_counts_wrapped() {
        assert_eq!(display_rows("hello", 80), 1);
        assert_eq!(display_rows("aaaa\nbbbb", 80), 2);
        assert_eq!(display_rows("aaaaaaaaaaaa", 5), 3); // 12 / 5 = 3 rows
        assert_eq!(display_rows("", 80), 1);
    }

    #[test]
    fn str_width_counts_wide_chars_double() {
        assert_eq!(str_width("abc"), 3);
        // 你好 = two wide chars = 4 display cols
        assert_eq!(str_width("你好"), 4);
        assert_eq!(str_width("a你b"), 4);
    }

    #[test]
    fn truncate_to_width_fits_display_columns() {
        // Fits → unchanged (boundary inclusive).
        assert_eq!(truncate_to_width("abc", 5), "abc");
        assert_eq!(truncate_to_width("abc", 3), "abc");
        // ASCII truncation reserves 1 col for the ellipsis.
        assert_eq!(truncate_to_width("abcdef", 4), "abc…");
        // CJK: 你好xy = 6 cols, cap 5 → budget 4 → 你(2)+好(2), x won't fit → "你好…"
        assert_eq!(truncate_to_width("你好xy", 5), "你好…");
        // CJK mid-width boundary: cap 3 → budget 2 → only 你 fits → "你…"
        assert_eq!(truncate_to_width("你好", 3), "你…");
    }
}

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

/// Display width of a char (0 for zero-width, 1 for most, 2 for wide
/// CJK/fullwidth/emoji). Approximates Unicode East Asian Width without
/// pulling in the unicode-width crate for the TUI.
pub fn char_width(ch: char) -> usize {
    let cp = ch as u32;
    // --- Zero-width: NUL, combining marks, joiners, variation selectors ---
    if cp == 0
        || (0x0300..=0x036F).contains(&cp) // combining diacritical marks
        || (0x200B..=0x200D).contains(&cp) // ZWSP, ZWNJ, ZWJ
        || (0xFE00..=0xFE0F).contains(&cp) // variation selectors
        || cp == 0xFEFF                    // BOM / zero-width no-break space
    {
        return 0;
    }
    // --- Wide (2 columns): CJK, fullwidth, and common emoji ranges ---
    if (0x1100..=0x115F).contains(&cp) // Hangul Jamo
        || (0x231A..=0x231B).contains(&cp) // watch, hourglass
        || (0x23E9..=0x23F3).contains(&cp) // media control emoji
        || (0x25FD..=0x25FE).contains(&cp) // small squares
        || (0x2614..=0x2615).contains(&cp) // umbrella, hot beverage
        || (0x2648..=0x2653).contains(&cp) // zodiac signs
        || (0x267F..=0x26FA).contains(&cp) // misc transport/symbols
        || (0x2702..=0x27B0).contains(&cp) // dingbats
        || (0x2934..=0x2935).contains(&cp) // arrows
        || (0x2B05..=0x2B55).contains(&cp) // arrows, geometric shapes
        || (0x2E80..=0xA4CF).contains(&cp) // CJK radicals -> Yi
        || (0xAC00..=0xD7A3).contains(&cp) // Hangul syllables
        || (0xF900..=0xFAFF).contains(&cp) // CJK compat ideographs
        || (0xFE30..=0xFE4F).contains(&cp) // CJK compat forms
        || (0xFF00..=0xFF60).contains(&cp) // fullwidth forms
        || (0xFFE0..=0xFFE6).contains(&cp) // fullwidth signs
        || (0x1F300..=0x1FAFF).contains(&cp) // emoji & symbols (SMP)
        || (0x20000..=0x3FFFD).contains(&cp) // CJK extension B and beyond
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

/// Insert a string at the cursor index, returning (new_text, new_idx). The
/// cursor advances by the number of chars in `s` (not bytes), staying on a
/// char boundary for multi-byte insertions.
pub fn insert_str(text: &str, idx: usize, s: &str) -> (String, usize) {
    let mut out = String::with_capacity(text.len() + s.len());
    let byte = byte_offset_for_char(text, idx);
    out.push_str(&text[..byte]);
    out.push_str(s);
    out.push_str(&text[byte..]);
    (out, idx + s.chars().count())
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

/// Delete the word before the cursor (readline `unix-word-rubout`, a.k.a.
/// Ctrl+W). Skips trailing whitespace, then deletes the preceding run of
/// non-whitespace. Does not cross newline boundaries — the deletion stops at
/// the start of the current line.
///
/// Returns `(new_text, new_idx)` or `None` if the cursor is already at the
/// start of the current line.
pub fn delete_word_back(text: &str, idx: usize) -> Option<(String, usize)> {
    if idx == 0 {
        return None;
    }
    // Find the start of the current line (char index after the last '\n'
    // before the cursor, or 0 if there is none).
    let chars: Vec<char> = text.chars().collect();
    let mut line_start = 0usize;
    for (i, &ch) in chars.iter().enumerate() {
        if i >= idx {
            break;
        }
        if ch == '\n' {
            line_start = i + 1;
        }
    }
    if idx <= line_start {
        return None;
    }
    let mut new_idx = idx;
    // 1. Skip whitespace backward (space, tab, etc. — but not '\n').
    while new_idx > line_start && is_word_whitespace(chars[new_idx - 1]) {
        new_idx -= 1;
    }
    // 2. Skip non-whitespace backward.
    while new_idx > line_start && !is_word_whitespace(chars[new_idx - 1]) {
        new_idx -= 1;
    }
    if new_idx == idx {
        return None;
    }
    let byte_start = byte_offset_for_char(text, new_idx);
    let byte_end = byte_offset_for_char(text, idx);
    let mut s = String::with_capacity(text.len() - (byte_end - byte_start));
    s.push_str(&text[..byte_start]);
    s.push_str(&text[byte_end..]);
    Some((s, new_idx))
}

fn is_word_whitespace(ch: char) -> bool {
    ch.is_whitespace() && ch != '\n'
}

/// A single visual (wrapped) row of the composer input. `start`..`end` is a
/// half-open char-index range; a row resulting from an explicit '\n' excludes
/// the newline (it only triggers the break).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualRow {
    pub start: usize,
    pub end: usize,
}

/// Split `input` into visual rows using word-boundary wrapping. The first
/// visual row is narrowed by `prompt_w` (the `❯ ` prefix occupies its leading
/// columns); every other row uses the full `inner_w`. Explicit '\n' always
/// starts a new row.
///
/// This is the **single source of truth** for composer wrapping: both
/// `render_composer` (which builds explicit `Line`s from these rows and
/// disables ratatui's own `Wrap`) and the cursor math derive from it, so the
/// rendered glyphs and the cursor position can never diverge.
pub fn wrap_rows(input: &str, inner_w: u16, prompt_w: u16) -> Vec<VisualRow> {
    let first_w = (inner_w.saturating_sub(prompt_w) as usize).max(1);
    let rest_w = (inner_w as usize).max(1);
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let mut rows: Vec<VisualRow> = Vec::new();
    if n == 0 {
        rows.push(VisualRow { start: 0, end: 0 });
        return rows;
    }
    let mut row_start = 0usize;
    let mut col = 0usize;
    // Char index just past the last wrappable whitespace on the current row.
    let mut last_break = 0usize;
    let mut row_idx = 0usize; // global visual row index (0 uses first_w)
    let mut i = 0usize;
    while i < n {
        let ch = chars[i];
        if ch == '\n' {
            rows.push(VisualRow { start: row_start, end: i });
            row_idx += 1;
            i += 1;
            row_start = i;
            col = 0;
            last_break = i;
            continue;
        }
        let cw = char_width(ch);
        let w = if row_idx == 0 { first_w } else { rest_w };
        if col + cw > w && col > 0 {
            // Overflow: prefer breaking at the last whitespace boundary so
            // whole words move to the next row; fall back to a mid-word break
            // (long word / no spaces). Re-evaluate the moved chars on the new
            // row by rewinding `i` to the break point.
            if last_break > row_start {
                rows.push(VisualRow { start: row_start, end: last_break });
                row_start = last_break;
                i = last_break;
            } else {
                rows.push(VisualRow { start: row_start, end: i });
                row_start = i;
            }
            row_idx += 1;
            col = 0;
            last_break = row_start;
            continue;
        }
        col += cw;
        i += 1;
        // A space/tab is a wrap candidate: a break may happen right after it.
        if ch == ' ' || ch == '\t' {
            last_break = i;
        }
    }
    rows.push(VisualRow { start: row_start, end: n });
    rows
}

/// Compute (row, col) display position from a char index, using the same
/// `wrap_rows` model as the renderer. The cursor at a row boundary (char index
/// equal to a row's `end`) belongs to that row's tail rather than the next
/// row's head, matching greedy-wrap cursor semantics.
pub fn cursor_row_col(input: &str, char_idx: usize, inner_w: u16, prompt_w: u16) -> (usize, usize) {
    let rows = wrap_rows(input, inner_w, prompt_w);
    let total = input.chars().count();
    let char_idx = char_idx.min(total);
    let mut row = 0usize;
    for (r, vr) in rows.iter().enumerate() {
        if vr.start <= char_idx && char_idx <= vr.end {
            row = r;
            break;
        }
    }
    let start = rows[row].start;
    let col: usize = input
        .chars()
        .skip(start)
        .take(char_idx.saturating_sub(start))
        .map(char_width)
        .sum();
    (row, col)
}

/// Move the cursor up/down by one visual (wrapped) row, preserving the display
/// column. Uses `wrap_rows` so movement correctly crosses soft-wrapped rows,
/// not just explicit newlines. Returns the original index if already at the
/// top/bottom visual row.
pub fn move_cursor_vertical(
    input: &str,
    char_idx: usize,
    direction: i32,
    inner_w: u16,
    prompt_w: u16,
) -> usize {
    if input.is_empty() {
        return char_idx;
    }
    let rows = wrap_rows(input, inner_w, prompt_w);
    let total = input.chars().count();
    let char_idx = char_idx.min(total);
    let chars: Vec<char> = input.chars().collect();
    // Find the current visual row (same rule as cursor_row_col).
    let mut cur = 0usize;
    for (r, vr) in rows.iter().enumerate() {
        if vr.start <= char_idx && char_idx <= vr.end {
            cur = r;
            break;
        }
    }
    let cur_start = rows[cur].start;
    let col: usize = chars[cur_start..char_idx].iter().map(|c| char_width(*c)).sum();
    let target = cur as i32 + direction;
    if target < 0 || target as usize >= rows.len() {
        return char_idx;
    }
    let trow = rows[target as usize];
    // Walk the target row forward accumulating width until we pass `col`,
    // landing on the closest char boundary.
    let mut actual = 0usize;
    let mut idx = trow.start;
    for (j, &ch) in chars[trow.start..trow.end].iter().enumerate().map(|(i, c)| (trow.start + i, c)) {
        let cw = char_width(ch);
        if actual + cw > col {
            break;
        }
        actual += cw;
        idx = j + 1;
    }
    idx
}

/// Count how many visual rows the input occupies. Derived from `wrap_rows`
/// so it matches the renderer exactly.
pub fn display_rows(input: &str, inner_w: u16, prompt_w: u16) -> u16 {
    (wrap_rows(input, inner_w, prompt_w).len() as u16).max(1)
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
        assert_eq!(cursor_row_col("hello", 0, 80, 0), (0, 0));
        assert_eq!(cursor_row_col("hello", 3, 80, 0), (0, 3));
        assert_eq!(cursor_row_col("hello", 5, 80, 0), (0, 5));
    }

    #[test]
    fn cursor_row_col_multi_line() {
        let input = "abc\ndef\nghi";
        assert_eq!(cursor_row_col(input, 0, 80, 0), (0, 0));
        assert_eq!(cursor_row_col(input, 3, 80, 0), (0, 3)); // before \n
        assert_eq!(cursor_row_col(input, 4, 80, 0), (1, 0)); // start of line 2
        assert_eq!(cursor_row_col(input, 7, 80, 0), (1, 3)); // before second \n
        assert_eq!(cursor_row_col(input, 8, 80, 0), (2, 0)); // start of line 3
    }

    #[test]
    fn cursor_row_col_soft_wrap() {
        // width 5: 5 chars per row; cursor past 5 wraps to next row.
        assert_eq!(cursor_row_col("aaaaaa", 4, 5, 0), (0, 4));
        assert_eq!(cursor_row_col("aaaaaa", 5, 5, 0), (0, 5));
        assert_eq!(cursor_row_col("aaaaaa", 6, 5, 0), (1, 1));
    }

    #[test]
    fn cursor_row_col_soft_wrap_edge_cases() {
        // 1. CJK wide chars (each width 2) cause a soft-wrap mid-text at
        //    width 5: 你好你好 = 8 display cols → wraps after 2 chars (4 cols)
        //    since the 3rd char (你, width 2) would exceed col 5.
        assert_eq!(cursor_row_col("你好你好", 0, 5, 0), (0, 0));
        assert_eq!(cursor_row_col("你好你好", 1, 5, 0), (0, 2));
        assert_eq!(cursor_row_col("你好你好", 2, 5, 0), (0, 4));
        assert_eq!(cursor_row_col("你好你好", 3, 5, 0), (1, 2));
        assert_eq!(cursor_row_col("你好你好", 4, 5, 0), (1, 4));

        // 2. Minimum width = 1: every char occupies its own row after the first.
        assert_eq!(cursor_row_col("abc", 0, 1, 0), (0, 0));
        assert_eq!(cursor_row_col("abc", 1, 1, 0), (0, 1));
        assert_eq!(cursor_row_col("abc", 2, 1, 0), (1, 1));
        assert_eq!(cursor_row_col("abc", 3, 1, 0), (2, 1));

        // 3. Empty input: loop never executes regardless of char_idx.
        assert_eq!(cursor_row_col("", 0, 80, 0), (0, 0));
        assert_eq!(cursor_row_col("", 5, 80, 0), (0, 0));

        // 4. CJK chars exactly fill the width, then an explicit newline resets.
        //    你好 = 4 cols, exactly fills width 4 (no soft-wrap since 4 > 4 is
        //    false), then '\n' moves to the next row.
        assert_eq!(cursor_row_col("你好\nabc", 0, 4, 0), (0, 0));
        assert_eq!(cursor_row_col("你好\nabc", 1, 4, 0), (0, 2));
        assert_eq!(cursor_row_col("你好\nabc", 2, 4, 0), (0, 4));
        assert_eq!(cursor_row_col("你好\nabc", 3, 4, 0), (1, 0));
        assert_eq!(cursor_row_col("你好\nabc", 4, 4, 0), (1, 1));

        // 5. Cursor at char_idx 0 is always (0, 0) on any input.
        assert_eq!(cursor_row_col("hello\nworld", 0, 80, 0), (0, 0));
        assert_eq!(cursor_row_col("你好", 0, 80, 0), (0, 0));

        // 6. char_idx beyond end of input: the loop processes every char.
        assert_eq!(cursor_row_col("ab", 100, 80, 0), (0, 2));
    }

    #[test]
    fn move_cursor_up_down() {
        let input = "aaaa\nbbbb\ncccc";
        // Index 2 = row 0 col 2 (display). Move down → row 1 col 2 = index 7.
        let idx = move_cursor_vertical(input, 2, 1, 80, 0);
        assert_eq!(cursor_row_col(input, idx, 80, 0), (1, 2));
        // Index 7 = row 1 col 2. Move up → row 0 col 2 = index 2.
        let idx = move_cursor_vertical(input, 7, -1, 80, 0);
        assert_eq!(cursor_row_col(input, idx, 80, 0), (0, 2));
        // Can't move up from row 0
        assert_eq!(move_cursor_vertical(input, 2, -1, 80, 0), 2);
        // Can't move down from last row
        assert_eq!(move_cursor_vertical(input, 10, 1, 80, 0), 10);
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
    fn wrap_rows_no_spaces_matches_greedy() {
        // Without spaces, word-wrap degenerates to greedy char wrap.
        let rows = wrap_rows("aaaaaa", 5, 0);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], VisualRow { start: 0, end: 5 });
        assert_eq!(rows[1], VisualRow { start: 5, end: 6 });
    }

    #[test]
    fn wrap_rows_breaks_at_word_boundary() {
        // "ab cdefgh" at width 5: word wrap moves "cdefgh" down after "ab ".
        // After the space-break, "cdefgh" has no further spaces so it wraps
        // greedily: 5 cols per row. This is exactly the case where greedy
        // char-wrap and word-wrap diverge on the FIRST row boundary.
        let rows = wrap_rows("ab cdefgh", 5, 0);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], VisualRow { start: 0, end: 3 });  // "ab "
        assert_eq!(rows[1], VisualRow { start: 3, end: 8 });  // "cdefg"
        assert_eq!(rows[2], VisualRow { start: 8, end: 9 });  // "h"
    }

    #[test]
    fn wrap_rows_preserves_trailing_space_at_break() {
        // trim:false semantics: the whitespace before a wrap stays on the
        // current row.
        let rows = wrap_rows("abcd ef", 5, 0);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], VisualRow { start: 0, end: 5 }); // "abcd "
        assert_eq!(rows[1], VisualRow { start: 5, end: 7 }); // "ef"
    }

    #[test]
    fn wrap_rows_explicit_newline() {
        let rows = wrap_rows("ab
cd", 80, 0);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], VisualRow { start: 0, end: 2 });
        assert_eq!(rows[1], VisualRow { start: 3, end: 5 });
    }

    #[test]
    fn wrap_rows_empty_input_single_row() {
        let rows = wrap_rows("", 80, 2);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], VisualRow { start: 0, end: 0 });
    }

    #[test]
    fn wrap_rows_first_row_narrowed_by_prompt() {
        // inner_w=5, prompt_w=2: row 0 holds 3 cols, rest hold 5.
        let rows = wrap_rows("aaaaaaaa", 5, 2);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], VisualRow { start: 0, end: 3 }); // 3 cols on row 0
        assert_eq!(rows[1], VisualRow { start: 3, end: 8 }); // 5 cols on row 1
    }

    #[test]
    fn cursor_row_col_tracks_word_wrap() {
        // The cursor must land on the SAME visual row the renderer produces.
        // For "ab cdefgh" width 5, 'e' (char idx 5) is on visual row 1 (after
        // the word-wrap), at column 2 (after "cd"). Greedy wrap would wrongly
        // place it on row 0 col 5.
        assert_eq!(cursor_row_col("ab cdefgh", 5, 5, 0), (1, 2));
        // Cursor before the wrap, right after the space (char idx 3) is at the
        // tail of row 0.
        assert_eq!(cursor_row_col("ab cdefgh", 3, 5, 0), (0, 3));
    }

    #[test]
    fn cursor_row_col_cjk_word_wrap() {
        // CJK + space: "你好 world" at width 6. 你好=4, then ' ' makes 5,
        // then 'w'(6) fills, 'o' overflows -> wrap. Word wrap keeps "world"
        // together if it fits on the next row.
        let rows = wrap_rows("你好 world", 6, 0);
        // row0: 你好 (4) + space(5) + w(6) -> 'o' overflows, break after w?
        // last_break is after the space (idx 3). 'w' at idx4 pushes col to 6,
        // 'o' at idx5 would be col7 > 6 -> wrap at last_break=3 -> row=[0,3).
        assert_eq!(rows[0], VisualRow { start: 0, end: 3 });
        // row1: "world" = w o r l d = 5 cols, fits in 6.
        assert_eq!(rows[1], VisualRow { start: 3, end: 8 });
    }

    #[test]
    fn display_rows_counts_word_wrap_rows() {
        // word-wrap gives 3 rows for this input; greedy would give 2.
        assert_eq!(display_rows("ab cdefgh", 5, 0), 3);
        assert_eq!(display_rows("ab cdefgh", 5, 2), 3);
    }

    #[test]
    fn move_cursor_vertical_crosses_soft_wrap() {
        // Multi-line input that ALSO soft-wraps. With width 5, line "aaaaa"
        // is one row; Up/Down must move across visual rows.
        let input = "aaaaa
bbbbb";
        // idx 2 = row 0 col 2. Down -> row 1 col 2.
        let idx = move_cursor_vertical(input, 2, 1, 80, 0);
        assert_eq!(cursor_row_col(input, idx, 80, 0), (1, 2));
        // Back up.
        let idx = move_cursor_vertical(input, idx, -1, 80, 0);
        assert_eq!(cursor_row_col(input, idx, 80, 0), (0, 2));
    }

    #[test]
    fn move_cursor_vertical_within_soft_wrap() {
        // "abcdef ghi" width 4: row0="abcd", row1="ef ", row2="ghi".
        // Wait — word wrap: 'a'1'b'2'c'3'd'4 -> 'e' overflows, no break yet
        // (last_break=0), mid-word break row=[0,4). row1: 'e'1'f'2' '3
        // last_break=7. 'g'3'h'4 -> 'i' overflows, break at last_break=7 ->
        // row=[4,7)="ef ". row2: "ghi".
        let input = "abcdefghi";
        let rows = wrap_rows(input, 4, 0);
        assert_eq!(rows.len(), 3);
        // idx 1 (col1 row0) -> down -> row1 col1 = idx 5 ('f')
        let idx = move_cursor_vertical(input, 1, 1, 4, 0);
        assert_eq!(cursor_row_col(input, idx, 4, 0).0, 1);
    }

    #[test]
    fn display_rows_counts_wrapped() {
        assert_eq!(display_rows("hello", 80, 0), 1);
        assert_eq!(display_rows("aaaa\nbbbb", 80, 0), 2);
        assert_eq!(display_rows("aaaaaaaaaaaa", 5, 0), 3); // 12 / 5 = 3 rows
        assert_eq!(display_rows("", 80, 0), 1);
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

    #[test]
    fn cursor_row_col_dual_width_prompt() {
        // inner_w=5, prompt_w=2: first visual row holds 3 cols, rest hold 5.
        assert_eq!(cursor_row_col("aaaaaa", 0, 5, 2), (0, 0));
        assert_eq!(cursor_row_col("aaaaaa", 3, 5, 2), (0, 3)); // fills first row
        assert_eq!(cursor_row_col("aaaaaa", 4, 5, 2), (1, 1)); // wraps to row 1
        assert_eq!(cursor_row_col("aaaaaa", 6, 5, 2), (1, 3)); // end of input
    }

    #[test]
    fn display_rows_dual_width_prompt() {
        // inner_w=5, prompt_w=2: first row holds 3, rest hold 5.
        assert_eq!(display_rows("aaaaaa", 5, 2), 2); // 3 + 3
        assert_eq!(display_rows("aaaaaaaa", 5, 2), 2); // 3 + 5
        assert_eq!(display_rows("aaaaaaaaaa", 5, 2), 3); // 3 + 5 + 2
    }

    #[test]
    fn delete_word_back_basic() {
        // "hello world|" → "hello |"
        let (s, i) = delete_word_back("hello world", 11).unwrap();
        assert_eq!(s, "hello ");
        assert_eq!(i, 6);
    }

    #[test]
    fn delete_word_back_single_word() {
        // "hello|" → ""
        let (s, i) = delete_word_back("hello", 5).unwrap();
        assert_eq!(s, "");
        assert_eq!(i, 0);
    }

    #[test]
    fn delete_word_back_trailing_whitespace() {
        // "hello   |" → "" (deletes word + trailing spaces, like bash)
        let (s, i) = delete_word_back("hello   ", 8).unwrap();
        assert_eq!(s, "");
        assert_eq!(i, 0);
    }

    #[test]
    fn delete_word_back_mid_word() {
        // "hello wo|rld" → "hello |rld"
        let (s, i) = delete_word_back("hello world", 8).unwrap();
        assert_eq!(s, "hello rld");
        assert_eq!(i, 6);
    }

    #[test]
    fn delete_word_back_after_space() {
        // "hello |world" → "|world" (deletes "hello " including the space)
        let (s, i) = delete_word_back("hello world", 6).unwrap();
        assert_eq!(s, "world");
        assert_eq!(i, 0);
    }

    #[test]
    fn delete_word_back_at_line_start_returns_none() {
        // Cursor at start of first line → nothing to delete
        assert!(delete_word_back("hello", 0).is_none());
    }

    #[test]
    fn delete_word_back_empty_input_returns_none() {
        assert!(delete_word_back("", 0).is_none());
    }

    #[test]
    fn delete_word_back_does_not_cross_newline() {
        // "line1\nline2|" → "line1\n" (only deletes "line2")
        let (s, i) = delete_word_back("line1\nline2", 11).unwrap();
        assert_eq!(s, "line1\n");
        assert_eq!(i, 6);
    }

    #[test]
    fn delete_word_back_at_second_line_start_returns_none() {
        // "line1\n|line2" → None (cursor at start of second line)
        assert!(delete_word_back("line1\nline2", 6).is_none());
    }

    #[test]
    fn delete_word_back_multibyte_chars() {
        // "你好 world|" → "你好 |"
        let (s, i) = delete_word_back("你好 world", 8).unwrap();
        assert_eq!(s, "你好 ");
        assert_eq!(i, 3);
    }

    #[test]
    fn delete_word_back_only_whitespace_before_cursor() {
        // "hello\n   |" → "hello\n" (deletes trailing spaces on current line)
        let (s, i) = delete_word_back("hello\n   ", 9).unwrap();
        assert_eq!(s, "hello\n");
        assert_eq!(i, 6);
    }

    #[test]
    fn delete_word_back_consecutive_presses() {
        // Simulate pressing Ctrl+W twice on "hello world"
        let (s1, i1) = delete_word_back("hello world", 11).unwrap();
        assert_eq!(s1, "hello ");
        assert_eq!(i1, 6);
        // Second press: "hello |" → "|"
        let (s2, i2) = delete_word_back(&s1, i1).unwrap();
        assert_eq!(s2, "");
        assert_eq!(i2, 0);
    }

    #[test]
    fn char_width_zero_width_combining_and_joiners() {
        assert_eq!(char_width('\u{0300}'), 0); // combining grave accent
        assert_eq!(char_width('\u{200B}'), 0); // ZWSP
        assert_eq!(char_width('\u{200C}'), 0); // ZWNJ
        assert_eq!(char_width('\u{200D}'), 0); // ZWJ
        assert_eq!(char_width('\u{FE0F}'), 0); // variation selector-16
        assert_eq!(char_width('\u{FEFF}'), 0); // BOM / zero-width no-break space
        // A combining mark adds no display width to its base char.
        assert_eq!(str_width("e\u{0300}"), 1); // decomposed e-grave = 1 column
        assert_eq!(str_width("a\u{0308}b"), 2); // a + combining diaeresis + b
    }

    #[test]
    fn char_width_extended_wide_emoji_ranges() {
        assert_eq!(char_width('⌚'), 2); // U+231A watch
        assert_eq!(char_width('⏩'), 2); // U+23E9 fast-forward
        assert_eq!(char_width('\u{25FD}'), 2); // U+25FD white medium small square
        assert_eq!(char_width('☔'), 2); // U+2614 umbrella with rain
        assert_eq!(char_width('♑'), 2); // U+2651 capricorn (zodiac)
        assert_eq!(char_width('♿'), 2); // U+267F wheelchair
        assert_eq!(char_width('✂'), 2); // U+2702 scissors
        assert_eq!(char_width('\u{2934}'), 2); // U+2934 arrow pointing rightwards
        assert_eq!(char_width('⭐'), 2); // U+2B50 star
        assert_eq!(char_width('⬅'), 2); // U+2B05 left arrow
        assert_eq!(char_width('📋'), 2); // U+1F4CB clipboard (existing range)
        assert_eq!(char_width('\u{20000}'), 2); // CJK extension B (plane 2)
    }
}

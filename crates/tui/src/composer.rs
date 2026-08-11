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
        || cp == 0xFEFF
    // BOM / zero-width no-break space
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
        || (0x20000..=0x3FFFD).contains(&cp)
    // CJK extension B and beyond
    {
        return 2;
    }
    1
}

/// Display width of a string: sum of per-char widths.
pub fn str_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// Whether a character must not be inserted directly into the composer.
fn is_corrupting_control(ch: char) -> bool {
    let cp = ch as u32;
    (cp <= 0x1F && cp != 0x09 && cp != 0x0A) // C0 except TAB and LF
        || cp == 0x7F // DEL
        || (0x80..=0x9F).contains(&cp) // C1 control characters
}

/// Normalize composer text through the shared terminal-safety boundary.
/// Newlines remain structural; tabs expand to spaces so cursor width and the
/// physical terminal can never disagree.
pub fn sanitize(s: &str) -> String {
    crate::terminal_text::sanitize_multiline(s).into_owned()
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
    if ch == '\t' {
        return insert_str(text, idx, "\t");
    }
    if is_corrupting_control(ch) {
        return (text.to_string(), idx);
    }
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
/// Maximum input size (256 KiB in chars) to prevent unbounded memory from
/// pasting huge text blobs. Inserts that would exceed this are silently
/// rejected (the input stays unchanged).
pub const MAX_INPUT_CHARS: usize = 256 * 1024;

pub fn insert_str(text: &str, idx: usize, s: &str) -> (String, usize) {
    // Strip terminal-corrupting control characters (C0/DEL/C1) so pasted text
    // can never scramble the display via `Span::raw`.
    let clean = sanitize(s);
    // C3: reject inserts that would exceed the input limit.
    let new_chars = text.chars().count().saturating_add(clean.chars().count());
    if new_chars > MAX_INPUT_CHARS {
        return (text.to_string(), idx);
    }
    let mut out = String::with_capacity(text.len() + clean.len());
    let byte = byte_offset_for_char(text, idx);
    out.push_str(&text[..byte]);
    out.push_str(&clean);
    out.push_str(&text[byte..]);
    (out, idx + clean.chars().count())
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

/// Word classification for readline-style movement: `Word` = alphanumeric or
/// underscore, `Punct` = other non-whitespace, `Space` = whitespace.
#[derive(PartialEq, Eq)]
enum WordKind {
    Word,
    Punct,
    Space,
}

fn classify_word(ch: char) -> WordKind {
    if ch.is_whitespace() {
        WordKind::Space
    } else if ch.is_alphanumeric() || ch == '_' {
        WordKind::Word
    } else {
        WordKind::Punct
    }
}

/// Readline `forward-word` (Alt+F): advance the cursor to the end of the
/// current or next word.
///
/// If the cursor rests on whitespace it skips that first; then it consumes
/// every character that shares the same [`WordKind`] as the character under
/// the cursor. Lands one past the last character of the word (the boundary
/// between the word and whatever follows).
pub fn forward_word(input: &str, cursor: usize) -> usize {
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = cursor.min(len);
    while i < len && chars[i].is_whitespace() {
        i += 1;
    }
    if i >= len {
        return len;
    }
    let kind = classify_word(chars[i]);
    while i < len && classify_word(chars[i]) == kind {
        i += 1;
    }
    i
}

/// Readline `backward-word` (Alt+B): move the cursor to the start of the
/// word preceding the cursor.
///
/// Steps back one position, skips any trailing whitespace, then consumes
/// every character sharing the same [`WordKind`]. Lands on the first
/// character of the word.
pub fn backward_word(input: &str, cursor: usize) -> usize {
    let chars: Vec<char> = input.chars().collect();
    if chars.is_empty() || cursor == 0 {
        return 0;
    }
    let mut i = (cursor.min(chars.len())) - 1;
    while i > 0 && chars[i].is_whitespace() {
        i -= 1;
    }
    if chars[i].is_whitespace() {
        return 0;
    }
    let kind = classify_word(chars[i]);
    while i > 0 && classify_word(chars[i - 1]) == kind {
        i -= 1;
    }
    i
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
            rows.push(VisualRow {
                start: row_start,
                end: i,
            });
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
                rows.push(VisualRow {
                    start: row_start,
                    end: last_break,
                });
                row_start = last_break;
                i = last_break;
            } else {
                rows.push(VisualRow {
                    start: row_start,
                    end: i,
                });
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
    rows.push(VisualRow {
        start: row_start,
        end: n,
    });
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

/// Translate the logical composer cursor into terminal coordinates. Kept
/// pure so rendering only applies the returned position to the frame.
#[allow(clippy::too_many_arguments)]
pub fn cursor_screen_position(
    area_x: u16,
    area_y: u16,
    input: &str,
    char_idx: usize,
    inner_w: u16,
    prompt_w: u16,
    scroll: u16,
    badge_h: u16,
) -> (u16, u16) {
    let (row, col) = cursor_row_col(input, char_idx, inner_w, prompt_w);
    let x = area_x + 1 + prompt_w + col as u16;
    let y = area_y + 1 + badge_h + (row as u16).saturating_sub(scroll);
    (x, y)
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
    let col: usize = chars[cur_start..char_idx]
        .iter()
        .map(|c| char_width(*c))
        .sum();
    let target = cur as i32 + direction;
    if target < 0 || target as usize >= rows.len() {
        return char_idx;
    }
    let trow = rows[target as usize];
    // Walk the target row forward accumulating width until we pass `col`,
    // landing on the closest char boundary.
    let mut actual = 0usize;
    let mut idx = trow.start;
    for (j, &ch) in chars[trow.start..trow.end]
        .iter()
        .enumerate()
        .map(|(i, c)| (trow.start + i, c))
    {
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
#[path = "composer_tests.rs"]
mod tests;

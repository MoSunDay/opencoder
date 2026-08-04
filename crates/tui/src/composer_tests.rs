//! Tests for the composer module.

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
    assert_eq!(rows[0], VisualRow { start: 0, end: 3 }); // "ab "
    assert_eq!(rows[1], VisualRow { start: 3, end: 8 }); // "cdefg"
    assert_eq!(rows[2], VisualRow { start: 8, end: 9 }); // "h"
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
    let rows = wrap_rows(
        "ab
cd", 80, 0,
    );
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

#[path = "composer_delete_tests.rs"]
mod delete;

#[path = "composer_word_tests.rs"]
mod word;

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

#[test]
fn insert_str_rejects_oversized_paste() {
    // C3: insert_str must silently reject pastes that would exceed
    // MAX_INPUT_CHARS to prevent unbounded memory growth.
    let big = "x".repeat(MAX_INPUT_CHARS);
    let (result, idx) = insert_str(&big, 0, "y");
    // Original text unchanged, cursor unchanged.
    assert_eq!(result.len(), big.len());
    assert_eq!(idx, 0);

    // Just under the limit is fine.
    let small = "x".repeat(MAX_INPUT_CHARS - 1);
    let (result, idx) = insert_str(&small, 0, "y");
    assert_eq!(result, format!("y{small}"));
    assert_eq!(idx, 1);
}

#[test]
fn sanitize_strips_c0_controls() {
    // BEL, BS, VT, FF, ESC and other C0 controls are removed.
    assert_eq!(sanitize("a\x07b"), "ab"); // BEL
    assert_eq!(sanitize("a\x08b"), "ab"); // BS
    assert_eq!(sanitize("a\x0Bb"), "ab"); // VT
    assert_eq!(sanitize("a\x0Cb"), "ab"); // FF
    assert_eq!(sanitize("a\x1Bb"), "ab"); // ESC
    assert_eq!(sanitize("\x00\x01\x1F"), "");
    // NUL handled explicitly too.
    assert_eq!(sanitize("\x00x"), "x");
}

#[test]
fn sanitize_keeps_tab_and_newline() {
    assert_eq!(sanitize("a\tb"), "a\tb");
    assert_eq!(sanitize("a\nb"), "a\nb");
    assert_eq!(sanitize("a\t\n\tb"), "a\t\n\tb");
}

#[test]
fn sanitize_strips_carriage_return() {
    // CR is removed so \r\n collapses to \n.
    assert_eq!(sanitize("a\rb"), "ab");
    assert_eq!(sanitize("a\r\nb"), "a\nb");
    assert_eq!(sanitize("\r"), "");
}

#[test]
fn sanitize_strips_del_and_c1() {
    assert_eq!(sanitize("a\x7Fb"), "ab"); // DEL
    assert_eq!(sanitize("\u{80}\u{9F}"), ""); // C1 range start and end
    assert_eq!(sanitize("x\u{85}y"), "xy"); // NEL (C1)
    assert_eq!(sanitize("x\u{9B}[31my"), "x[31my"); // CSI
}

#[test]
fn sanitize_preserves_normal_text() {
    assert_eq!(sanitize("hello world"), "hello world");
    assert_eq!(sanitize("你好世界"), "你好世界"); // CJK
    assert_eq!(sanitize("👋🌍"), "👋🌍"); // emoji
    assert_eq!(sanitize("café résumé"), "café résumé"); // accents
    assert_eq!(sanitize(""), "");
}

#[test]
fn insert_str_sanitizes_pasted_text() {
    // A pasted blob containing terminal-corrupting control codes comes
    // out clean: the control bytes (BEL, ESC) are stripped while the
    // printable chars survive; the cursor advances past only those.
    let (text, idx) = insert_str("ab", 1, "\x07he\x1B[31mllo");
    // sanitize drops \x07 and \x1B -> "he[31mllo" (9 chars)
    assert_eq!(text, "ahe[31mllob");
    assert_eq!(idx, 10);
}

#[test]
fn insert_char_skips_control_chars() {
    // A corrupting control char typed/pasted as a single char is dropped:
    // the text and cursor are returned unchanged.
    let (text, idx) = insert_char("abc", 1, '\x07');
    assert_eq!(text, "abc");
    assert_eq!(idx, 1);

    let (text, idx) = insert_char("abc", 1, '\u{9B}');
    assert_eq!(text, "abc");
    assert_eq!(idx, 1);

    // A normal printable char still inserts as before.
    let (text, idx) = insert_char("abc", 1, 'X');
    assert_eq!(text, "aXbc");
    assert_eq!(idx, 2);
}

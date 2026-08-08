use super::*;

fn place_cursor(
    frame: &mut Frame,
    area: Rect,
    input: &str,
    cursor_idx: usize,
    inner_w: u16,
    prompt_w: u16,
    scroll: u16,
) {
    frame.set_cursor_position(crate::composer::cursor_screen_position(
        area.x, area.y, input, cursor_idx, inner_w, prompt_w, scroll,
    ));
}

/// Row 0 cursor: x = composer.x + border + prompt_w + col.
#[test]
fn place_cursor_row_zero() {
    let backend = TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            place_cursor(f, Rect::new(0, 5, 40, 4), "hello", 2, 36, 2, 0);
        })
        .unwrap();
    // row=0, col=2 → x = 0+1+2+2 = 5, y = 5+1+0-0 = 6.
    terminal.backend_mut().assert_cursor_position((5, 6));
}

/// Cursor on the second physical line (after an explicit `\n`): no prompt
/// offset, so x = composer.x + border + col.
#[test]
fn place_cursor_second_line() {
    let backend = TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            place_cursor(f, Rect::new(0, 5, 40, 4), "hello\nworld", 8, 36, 2, 0);
        })
        .unwrap();
    // cursor_row_col("hello\nworld", 8, 36, 2) = (1, 2)
    // row>0, uniform prompt_w → x = 0+1+2+2 = 5, y = 5+1+1-0 = 7.
    terminal.backend_mut().assert_cursor_position((5, 7));
}

/// Soft-wrap at the inner width boundary advances the cursor to the next
/// visual row even without an explicit newline.
#[test]
fn place_cursor_soft_wrap_advances_row() {
    let backend = TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            place_cursor(f, Rect::new(0, 5, 40, 4), "aaaaaa", 6, 5, 2, 0);
        })
        .unwrap();
    // cursor_row_col("aaaaaa", 6, 5, 2) = (1, 3)
    // row_w = 5-2 = 3; uniform prompt_w → x = 0+1+2+3 = 6, y = 5+1+1-0 = 7.
    terminal.backend_mut().assert_cursor_position((6, 7));
}

/// Scrolling the composer shifts the cursor's screen row by `scroll`.
#[test]
fn place_cursor_with_scroll() {
    let backend = TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            place_cursor(
                f,
                Rect::new(0, 5, 40, 4),
                "line1\nline2\nline3",
                12,
                80,
                2,
                1,
            );
        })
        .unwrap();
    // cursor_row_col("line1\nline2\nline3", 12, 80, 2) = (2, 0)
    // row>0, uniform prompt_w → x = 0+1+2+0 = 3, y = 5+1+2-1 = 7.
    terminal.backend_mut().assert_cursor_position((3, 7));
}

/// Cross-check (Fix #4): text with a space so WORD-wrap diverges from the
/// old greedy char-wrap. The rendered buffer must show the word-wrap
/// ("ab " on the first content row, "cdefgh" wrapped to the next), AND the
/// cursor computed by `place_cursor` must land on the same visual row.
#[test]
fn composer_word_wrap_renders_and_cursor_aligns() {
    // composer width 12 -> inner_w=8 (after borders), prompt_w=2 -> first_w=6.
    let backend = TestBackend::new(12, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    let input = "ab cdefgh";
    terminal
        .draw(|f| {
            render_composer(f, Rect::new(0, 0, 12, 6), input, 0, 8, 2, &[], false, None);
        })
        .unwrap();
    let buf = terminal.backend().buffer();
    let r1 = row_text(buf, 1, 12);
    let r2 = row_text(buf, 2, 12);
    // Word-wrap broke at the space: "ab " (with prompt) on row 1, "cdefgh" on
    // row 2. Greedy char-wrap would have put "ab cdef" on row 1 and "gh" on
    // row 2 — which is exactly the misalignment this fixes.
    assert!(r1.contains("ab"), "row1 should start with prompt+ab: {r1}");
    assert!(
        !r1.contains("cdefgh"),
        "cdefgh must NOT stay on the first content row: {r1}"
    );
    assert!(r2.contains("cdefgh"), "cdefgh must wrap to row 2: {r2}");

    // Cursor at char_idx 5 ('e') is on visual row 1: cursor_row_col gives
    // (1, 2), so x = border + prompt_w + col = 1 + 2 + 2 = 5, y = border + row = 1 + 1 = 2.
    terminal
        .draw(|f| {
            place_cursor(f, Rect::new(0, 0, 12, 6), input, 5, 8, 2, 0);
        })
        .unwrap();
    terminal.backend_mut().assert_cursor_position((5, 2));
}

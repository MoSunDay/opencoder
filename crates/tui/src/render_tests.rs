use super::*;
use crate::chat::ChatView;
use opencoder_session::SessionEvent;
use ratatui::backend::TestBackend;

fn thinking_view() -> ChatView {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("think-a-1\nthink-a-2".into()));
    v.apply(&SessionEvent::TextDelta("answer".into()));
    v.apply(&SessionEvent::Done);
    v
}

/// A collapsed thinking header at the top is visible at scroll 0 and gets
/// a full-width hit rect on its header row.
#[test]
fn collapsed_header_visible_gets_hit_rect() {
    let v = thinking_view();
    let lines = v.flatten();
    // Header is the first line (line index 0).
    let headers = v.thinking_headers();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].header_line_idx, 0);

    let mut hits = Vec::new();
    record_thinking_hits(&v, &lines, 40, 0, 10, 1, 2, &mut hits);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].block_idx, headers[0].block_idx);
    // screen_y = y0 + (0 - 0) = 2; full text width.
    assert_eq!(hits[0].rect, Rect::new(1, 2, 40, 1));
}

/// Expanding the thinking block grows its rendered lines but the header
/// stays at the same screen row (row 0 → screen y0).
#[test]
fn expanded_header_row_unchanged() {
    let mut v = thinking_view();
    v.toggle_thinking_at(v.thinking_headers()[0].block_idx);
    let lines = v.flatten();
    let mut hits = Vec::new();
    record_thinking_hits(&v, &lines, 40, 0, 10, 1, 2, &mut hits);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].rect, Rect::new(1, 2, 40, 1));
    // Content lines are now present in the flattened output.
    assert!(lines
        .iter()
        .any(|l| { l.spans.iter().any(|s| s.content.contains("think-a-1")) }));
}

/// Scrolling past the header removes its hit rect (header scrolled out of
/// view above).
#[test]
fn header_scrolled_above_is_not_hittable() {
    let v = thinking_view();
    let lines = v.flatten();
    let mut hits = Vec::new();
    // scroll_y = 1 pushes the row-0 header above the viewport.
    record_thinking_hits(&v, &lines, 40, 1, 10, 1, 2, &mut hits);
    assert!(
        hits.is_empty(),
        "header above viewport should not be hittable"
    );
}

/// No thinking blocks ⇒ no work and no hits.
#[test]
fn no_thinking_blocks_means_no_hits() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("just text".into()));
    v.apply(&SessionEvent::Done);
    let lines = v.flatten();
    let mut hits = Vec::new();
    record_thinking_hits(&v, &lines, 40, 0, 10, 1, 2, &mut hits);
    assert!(hits.is_empty());
}

/// in_rect matches a click on the header row and misses other rows.
#[test]
fn hit_rect_matches_click_on_header_row() {
    let v = thinking_view();
    let lines = v.flatten();
    let mut hits = Vec::new();
    record_thinking_hits(&v, &lines, 40, 0, 10, 1, 2, &mut hits);
    let rect = hits[0].rect;
    // Click anywhere on the header row (y == 2) within x..x+width hits.
    assert!(in_rect(rect, 5, 2));
    assert!(in_rect(rect, 1, 2));
    // Adjacent rows do not hit.
    assert!(!in_rect(rect, 5, 1));
    assert!(!in_rect(rect, 5, 3));
}

/// Collect the rendered text of a single buffer row into a String by
/// concatenating every cell's symbol. Wide-char spacer cells (reset to a
/// space by ratatui) contribute a space, so callers should check for ASCII
/// substrings or individual wide chars rather than contiguous CJK runs.
fn row_text(buf: &ratatui::buffer::Buffer, y: u16, width: u16) -> String {
    let mut s = String::new();
    for x in 0..width {
        if let Some(cell) = buf.cell((x, y)) {
            s.push_str(cell.symbol());
        }
    }
    s
}

// ----- Guard (A): status bar no longer shows the word "opencoder" -----

/// The status bar renders model / agent / dir / ctx but must NOT contain the
/// brand name "opencoder" anywhere (regression guard for the de-branding).
#[test]
fn status_bar_omits_branding() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render_status(
                f, area, false, "", "glm-4.6", "act", 0, 0, 200000,
            );
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    assert!(
        !row.to_lowercase().contains("opencoder"),
        "status bar must not contain branding; got: {row}"
    );
    assert!(row.contains("glm-4.6"), "model should appear; got: {row}");
    assert!(
        row.contains("[act]"),
        "agent chip should appear; got: {row}"
    );
}

/// While running, the status bar shows the status text plus the first braille
/// spinner frame, and still omits the brand name.
#[test]
fn status_bar_running_shows_spinner_and_status() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render_status(
                f, area, true, "thinking", "glm-4.6", "act", 0, 0, 200000,
            );
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    assert!(
        row.contains("thinking"),
        "status text should appear; got: {row}"
    );
    assert!(
        row.contains('\u{280b}'),
        "first spinner frame should appear; got: {row}"
    );
    assert!(
        !row.to_lowercase().contains("opencoder"),
        "status bar must not contain branding; got: {row}"
    );
}


// ----- Guard: skill badge removed from status bar -----

/// The status bar must NOT render a `skill:` badge (removed per user request
/// — the skill is no longer surfaced in the bottom bar; only the echoed text
/// in the body carries the {$name} token verbatim).
#[test]
fn status_bar_has_no_skill_badge() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render_status(
                f, area, false, "", "glm-4.6", "act", 0, 0, 200000,
            );
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    assert!(
        !row.contains("skill:"),
        "status bar must not contain skill badge; got: {row}"
    );
}

// ----- Guard: steer/queue counters removed from status bar; ctx% present -----

/// The status bar no longer carries the steer/queue counters but DOES show the
/// ctx% indicator (moved from the body's reserved bottom row into the status
/// bar). Guards against accidental re-introduction of steer/queue.
#[test]
fn status_bar_has_no_steer_queue_or_ctx() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render_status(
                f, area, true, "thinking", "glm-4.6", "act", 0, 0, 200000,
            );
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    assert!(
        !row.contains("steer:"),
        "status bar must not show steer counter; got: {row}"
    );
    assert!(
        !row.contains("queue:"),
        "status bar must not show queue counter; got: {row}"
    );
    assert!(
        row.contains("ctx"),
        "status bar should show ctx indicator; got: {row}"
    );
}

/// The queue panel renders steer items with the `↳ steer` prefix and queue
/// items with `[queued]`, and caps display at 3 rows.
#[test]
fn queue_panel_renders_steer_and_queue_rows() {
    use crate::queue_panel::QueueBtn;
    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    let steers: Vec<(i64, String)> = vec![(1, "fix bug".into())];
    let queues: Vec<(i64, String)> = vec![(2, "run lint".into())];
    let mut btns: Vec<QueueBtn> = Vec::new();
    terminal
        .draw(|f| {
            let area = f.area();
            render_queue_panel(f, area, &steers, &queues, &mut btns);
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let row0 = row_text(buf, 0, 80);
    let row1 = row_text(buf, 1, 80);
    assert!(
        row0.contains("steer") && row0.contains("fix bug"),
        "steer row missing: {row0}"
    );
    assert!(
        row1.contains("queued") && row1.contains("run lint"),
        "queue row missing: {row1}"
    );
}

/// Steer rows register two hit-rects (Delete + Submit); queue rows register
/// three (Up + Down + Delete). Steer rows are now clickable (seq: Some).
#[test]
fn queue_panel_registers_correct_btns_for_steer_and_queue() {
    use crate::queue_panel::{QueueBtn, QueueBtnAction};
    let backend = TestBackend::new(80, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    let steers: Vec<(i64, String)> = vec![(10, "fix bug".into())];
    let queues: Vec<(i64, String)> = vec![(20, "run lint".into())];
    let mut btns: Vec<QueueBtn> = Vec::new();
    terminal
        .draw(|f| {
            let area = f.area();
            render_queue_panel(f, area, &steers, &queues, &mut btns);
        })
        .unwrap();

    // Steer row (seq=10) should have Delete + Submit.
    let steer_btns: Vec<_> = btns.iter().filter(|b| b.seq == 10).collect();
    assert_eq!(steer_btns.len(), 2, "steer row should have 2 buttons");
    assert!(
        steer_btns
            .iter()
            .any(|b| b.action == QueueBtnAction::Delete),
        "steer row must have a Delete button"
    );
    assert!(
        steer_btns
            .iter()
            .any(|b| b.action == QueueBtnAction::Submit),
        "steer row must have a Submit button"
    );
    // Steer row must NOT have Up or Down.
    assert!(
        !steer_btns
            .iter()
            .any(|b| b.action == QueueBtnAction::Up || b.action == QueueBtnAction::Down),
        "steer row must not have Up/Down buttons"
    );

    // Queue row (seq=20) should have Up + Down + Delete.
    let queue_btns: Vec<_> = btns.iter().filter(|b| b.seq == 20).collect();
    assert_eq!(queue_btns.len(), 3, "queue row should have 3 buttons");
    assert!(
        queue_btns.iter().any(|b| b.action == QueueBtnAction::Up),
        "queue row must have an Up button"
    );
    assert!(
        queue_btns
            .iter()
            .any(|b| b.action == QueueBtnAction::Down),
        "queue row must have a Down button"
    );
    assert!(
        queue_btns
            .iter()
            .any(|b| b.action == QueueBtnAction::Delete),
        "queue row must have a Delete button"
    );
    // Queue row must NOT have Submit.
    assert!(
        !queue_btns
            .iter()
            .any(|b| b.action == QueueBtnAction::Submit),
        "queue row must not have a Submit button"
    );
}

// ----- Guard (B): composer rendering with multi-line input -----


/// The composer renders a `❯ ` prompt on the first line, the first input
/// segment after it, subsequent lines without a prompt, and a follow label
/// on the top border row.
#[test]
fn composer_renders_prompt_and_multiline_text() {
    let backend = TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_composer(
                f,
                Rect::new(0, 0, 40, 5),
                "hello\nworld",
                0,
                38, // inner_w: 40 - 2 borders
                2,  // prompt_w: "❯ "
            );
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    // Prompt glyph lands at the first inner cell (border=1).
    assert_eq!(buf[(1, 1)].symbol(), "\u{276f}", "prompt glyph at (1,1)");
    let row1 = row_text(buf, 1, 40);
    let row2 = row_text(buf, 2, 40);
    assert!(
        row1.contains('\u{276f}'),
        "prompt should appear on row 1; got: {row1}"
    );
    assert!(
        row1.contains("hello"),
        "hello should appear on row 1; got: {row1}"
    );
    assert!(
        row2.contains("world"),
        "world should appear on row 2; got: {row2}"
    );
}

/// When not following, the body's bottom-border row shows the `⬇` (U+2B07)
/// follow indicator and exports its hit rect via `jump_btn`.
#[test]
fn body_follow_indicator_when_not_following() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hello".into()));
    v.apply(&SessionEvent::Done);

    let backend = TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut jump_btn: Option<Rect> = None;
    let mut top_btn: Option<Rect> = None;
    let mut body_out: Option<Rect> = None;
    let mut scroll = 0u16;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                &v,
                "test",
                &mut scroll,
                false,
                0,
                &mut body_out,
                &mut jump_btn,
                &mut top_btn,
                &mut Vec::new(),
                &mut Vec::new(),
                None,
            );
        })
        .unwrap();

    // Bottom border row is the last row of the area.
    let area = terminal.backend().buffer().area;
    let bottom_row = area.bottom() - 1;
    let row = row_text(terminal.backend().buffer(), bottom_row, area.width);
    assert!(
        row.contains('\u{2b07}'),
        "follow arrow ⬇ should appear on bottom border; got: {row}"
    );
    assert!(
        jump_btn.is_some(),
        "jump_btn should be set to a rect when not following"
    );
}

/// When following, the body's bottom-border row shows the "跟随中…" label
/// and `jump_btn` is `None` (no clickable target while already at the bottom).
#[test]
fn body_follow_label_when_following() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hello".into()));
    v.apply(&SessionEvent::Done);

    let backend = TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut jump_btn: Option<Rect> = None;
    let mut top_btn: Option<Rect> = None;
    let mut body_out: Option<Rect> = None;
    let mut scroll = 0u16;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                &v,
                "test",
                &mut scroll,
                true,
                0,
                &mut body_out,
                &mut jump_btn,
                &mut top_btn,
                &mut Vec::new(),
                &mut Vec::new(),
                None,
            );
        })
        .unwrap();

    let area = terminal.backend().buffer().area;
    let bottom_row = area.bottom() - 1;
    let row = row_text(terminal.backend().buffer(), bottom_row, area.width);
    assert!(
        row.contains('跟') && row.contains('随'),
        "follow label should appear on bottom border; got: {row}"
    );
    assert!(
        jump_btn.is_none(),
        "jump_btn should be None when following"
    );
}

/// When scrolled past the top, the body's top-border row shows the `⬆`
/// (U+2B06) jump-to-top indicator and exports its hit rect via `top_btn`.
/// Unlike the bottom indicator it carries no "跟随中"-style label.
#[test]
fn body_top_arrow_when_scrolled_down() {
    let mut v = ChatView::default();
    // Plenty of lines so the content overflows a 40x10 window.
    let body = (0..40).map(|i| format!("line {i}\n")).collect::<String>();
    v.apply(&SessionEvent::TextDelta(body));
    v.apply(&SessionEvent::Done);

    let backend = TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut jump_btn: Option<Rect> = None;
    let mut top_btn: Option<Rect> = None;
    let mut body_out: Option<Rect> = None;
    let mut scroll = 5u16;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                &v,
                "test",
                &mut scroll,
                false,
                0,
                &mut body_out,
                &mut jump_btn,
                &mut top_btn,
                &mut Vec::new(),
                &mut Vec::new(),
                None,
            );
        })
        .unwrap();

    let area = terminal.backend().buffer().area;
    let top_row = row_text(terminal.backend().buffer(), area.y, area.width);
    assert!(
        top_row.contains('\u{2b06}'),
        "jump-to-top arrow ⬆ should appear on top border; got: {top_row}"
    );
    assert!(
        top_btn.is_some(),
        "top_btn should be set to a rect when scrolled past the top"
    );
}

/// At the very top (scroll 0) no jump-to-top arrow is shown and `top_btn` is
/// `None` — nothing to scroll up to.
#[test]
fn body_no_top_arrow_when_at_top() {
    let mut v = ChatView::default();
    let body = (0..40).map(|i| format!("line {i}\n")).collect::<String>();
    v.apply(&SessionEvent::TextDelta(body));
    v.apply(&SessionEvent::Done);

    let backend = TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut jump_btn: Option<Rect> = None;
    let mut top_btn: Option<Rect> = None;
    let mut body_out: Option<Rect> = None;
    let mut scroll = 0u16;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                &v,
                "test",
                &mut scroll,
                false,
                0,
                &mut body_out,
                &mut jump_btn,
                &mut top_btn,
                &mut Vec::new(),
                &mut Vec::new(),
                None,
            );
        })
        .unwrap();

    let area = terminal.backend().buffer().area;
    let top_row = row_text(terminal.backend().buffer(), area.y, area.width);
    assert!(
        !top_row.contains('\u{2b06}'),
        "no jump-to-top arrow when at the top; got: {top_row}"
    );
    assert!(top_btn.is_none(), "top_btn should be None when at the top");
}

// ----- Guard (B): cursor placement with multi-line + soft-wrap input -----

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
    // row>0 → x = 0+1+2 = 3, y = 5+1+1-0 = 7.
    terminal.backend_mut().assert_cursor_position((3, 7));
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
    // first_w = 5-2 = 3, rest_w = 5; row>0 → x = 0+1+3 = 4, y = 5+1+1-0 = 7.
    terminal.backend_mut().assert_cursor_position((4, 7));
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
    // row>0 → x = 0+1+0 = 1, y = 5+1+2-1 = 7.
    terminal.backend_mut().assert_cursor_position((1, 7));
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
            render_composer(f, Rect::new(0, 0, 12, 6), input, 0, 8, 2);
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
    // (1, 2), so x = border + col = 1 + 2 = 3, y = border + row = 1 + 1 = 2.
    terminal
        .draw(|f| {
            place_cursor(f, Rect::new(0, 0, 12, 6), input, 5, 8, 2, 0);
        })
        .unwrap();
    terminal.backend_mut().assert_cursor_position((3, 2));
}

/// Issue #6: the `[agent]` status chip is Yellow in plan mode and Cyan
/// for every other agent. Guards against a regression to the old uniform
/// Magenta.
#[test]
fn agent_chip_color_is_yellow_for_plan_cyan_otherwise() {
    assert_eq!(agent_chip_fg("plan"), Color::Yellow);
    assert_eq!(agent_chip_fg("act"), Color::Cyan);
    assert_eq!(agent_chip_fg("explore"), Color::Cyan);
    assert_eq!(agent_chip_fg(""), Color::Cyan);
}

/// Issue #6: the plan/act mode-flash chip background is Yellow for plan,
/// Cyan for act. Both the agent chip and the flash share the same theme
/// mapping, so they never visually disagree.
#[test]
fn mode_flash_bg_matches_plan_yellow_act_cyan() {
    assert_eq!(mode_flash_bg(true), Color::Yellow);
    assert_eq!(mode_flash_bg(false), Color::Cyan);
    // The two theme helpers agree on plan/act, so the chip and flash
    // always render the same hue.
    assert_eq!(agent_chip_fg("plan"), mode_flash_bg(true));
    assert_eq!(agent_chip_fg("act"), mode_flash_bg(false));
}

/// Issue #5 core invariant: while a preamble block is WITHHELD (multiple
/// subagents running), the `header_line_idx` values reported by
/// `thinking_headers()` and `subagent_headers()` must exactly match the
/// line indices in `flatten_with()` where those headers actually render.
/// If any of the `is_withheld` guards in those three functions drift out
/// of sync, a header index would point at the wrong row and mouse clicks
/// would land on the wrong block.
#[test]
fn header_line_indices_aligned_with_flatten_while_withheld() {
    let mut v = ChatView::default();
    // Preamble assistant text — withheld once 2 subagents run. Its "say:"
    // header + 2 content lines mean a stale (non-skipping) accounting
    // would shift every later header by 3 rows.
    v.apply(&SessionEvent::TextDelta(
        "preamble line one\npreamble line two".into(),
    ));
    v.apply(&SessionEvent::SubagentStart {
        id: "a".into(),
        kind: "explore".into(),
        prompt: "pa".into(),
        child_session_id: "ca".into(),
    });
    v.apply(&SessionEvent::SubagentStart {
        id: "b".into(),
        kind: "explore".into(),
        prompt: "pb".into(),
        child_session_id: "cb".into(),
    });
    // Thinking block after the subagents: its header_line_idx is the
    // canary — if the withheld preamble were counted it would overshoot.
    v.apply(&SessionEvent::ReasoningDelta(
        "post\ndispatch\nanalysis".into(),
    ));

    assert!(
        v.hidden_assistant_idx.is_some(),
        "preamble must be withheld"
    );
    assert_eq!(v.subagents_running, 2);
    let flat = v.flatten_with(0);

    let line_text =
        |idx: usize| -> String { flat[idx].spans.iter().map(|s| s.content.clone()).collect() };
    // Every thinking header points at a flatten line containing "Thinking".
    let th = v.thinking_headers();
    assert!(!th.is_empty());
    for h in &th {
        let txt = line_text(h.header_line_idx);
        assert!(
            txt.contains("Thinking"),
            "thinking header_line_idx {} -> {:?}",
            h.header_line_idx,
            txt,
        );
    }
    // Every subagent header points at a flatten line containing "subagent".
    let sh = v.subagent_headers();
    assert_eq!(sh.len(), 2);
    for h in &sh {
        let txt = line_text(h.header_line_idx);
        assert!(
            txt.contains("subagent"),
            "subagent header_line_idx {} -> {:?}",
            h.header_line_idx,
            txt,
        );
    }
    // No two headers collide on the same rendered line.
    let mut all_idx: Vec<usize> = th.iter().map(|h| h.header_line_idx).collect();
    all_idx.extend(sh.iter().map(|h| h.header_line_idx));
    let mut sorted = all_idx.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), all_idx.len(), "collide: {:?}", all_idx);
    // The withheld preamble contributes ZERO lines to flatten.
    for (i, line) in flat.iter().enumerate() {
        let txt: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(
            !txt.contains("preamble line"),
            "line {i}: withheld preamble leaked: {:?}",
            txt,
        );
    }
}

#[test]
fn status_chip_width_accounts_for_wide_emoji() {
    // Two emoji = 4 display columns but only 2 chars. With the old
    // chars().count() the chip rectangle was 2 columns too narrow, so the
    // second emoji was clipped out of the render entirely.
    let backend = TestBackend::new(60, 1);
    let mut terminal = Terminal::new(backend).unwrap();
    let text = "📋🎉";
    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, 60, 1);
            render_status_chip(f, area, text, Color::Green);
        })
        .unwrap();
    let row = row_text(terminal.backend().buffer(), 0, 60);
    assert!(row.contains('📋'), "first emoji missing; got: {row}");
    assert!(
        row.contains('🎉'),
        "second emoji was clipped — chip width did not account for wide chars; got: {row}"
    );
}

#[test]
fn status_bar_shows_ctx_percent() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_status(
                f, f.area(), false, "", "glm-4.6", "act", 0, 5000, 200000,
            );
        })
        .unwrap();

    let row = row_text(terminal.backend().buffer(), 0, 120);
    assert!(row.contains("ctx"), "status bar should show ctx; got: {row}");
    assert!(row.contains('%'), "status bar should show percent; got: {row}");
    assert!(
        row.contains("5K"),
        "should show compact used tokens; got: {row}"
    );
    assert!(
        row.contains("200K"),
        "should show compact limit tokens; got: {row}"
    );
}

/// High context usage renders the status bar ctx% indicator in red.
#[test]
fn status_bar_ctx_red_at_high_usage() {
    let backend = TestBackend::new(120, 3);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            render_status(
                f, f.area(), false, "", "glm-4.6", "act", 0, 180000, 200000,
            );
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let area = buf.area;
    let row = row_text(buf, 0, area.width);
    let ctx_col = row.find("ctx").expect("ctx should be present");
    let cell = buf
        .cell((ctx_col as u16, 0))
        .expect("cell at ctx");
    assert_eq!(cell.fg, Color::Red, "high usage should be red; got: {row}");
}

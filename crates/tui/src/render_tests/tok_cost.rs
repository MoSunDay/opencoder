//! `[tok cost]` corner: the body block's bottom-border left title shows the
//! session-lifetime token total, coexists with the right-bottom follow
//! indicator, hides in copy mode, and drops on narrow widths.

use super::*;
use crate::chat::ChatView;
use opencoder_session::SessionEvent;
use ratatui::backend::TestBackend;

fn draw(
    v: &ChatView,
    width: u16,
    height: u16,
    follow: bool,
    copy_mode: bool,
    tail_ms: u64,
) -> ratatui::buffer::Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut scroll = 0u32;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                v,
                &Line::raw("test"),
                &mut scroll,
                follow,
                0,
                0,
                &mut None,
                &mut None,
                &mut None,
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut None,
                true,
                tail_ms,
                copy_mode,
                None,
            );
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

fn full_text(buf: &ratatui::buffer::Buffer) -> String {
    (0..buf.area.height)
        .flat_map(|y| {
            row_text(buf, y, buf.area.width)
                .chars()
                .collect::<Vec<char>>()
        })
        .collect()
}

#[test]
fn tok_cost_corner_defaults_to_floor_on_bottom_border() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hi".into()));
    v.apply(&SessionEvent::Done);
    let buf = draw(&v, 60, 10, true, false, 0);
    let bottom = row_text(&buf, buf.area.bottom() - 1, buf.area.width);
    assert!(
        bottom.contains("[tok cost 0]"),
        "empty session shows 0 on the bottom border; got: {bottom}"
    );
}

#[test]
fn tok_cost_corner_shows_accumulated_total() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hi".into()));
    v.apply(&SessionEvent::LlmUsage {
        total_tokens: 700_000,
        input_tokens: 600_000,
        output_tokens: 100_000,
    });
    v.apply(&SessionEvent::LlmUsage {
        total_tokens: 534_567,
        input_tokens: 400_000,
        output_tokens: 134_567,
    });
    v.apply(&SessionEvent::Done);
    let buf = draw(&v, 60, 10, true, false, 0);
    let bottom = row_text(&buf, buf.area.bottom() - 1, buf.area.width);
    assert!(
        bottom.contains("[tok cost 1.235m]"),
        "injected totals format as millions; got: {bottom}"
    );
}

#[test]
fn tok_cost_coexists_with_right_bottom_indicator() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hi".into()));
    v.apply(&SessionEvent::Done);
    let buf = draw(&v, 40, 10, false, false, 0);
    let bottom = row_text(&buf, buf.area.bottom() - 1, buf.area.width);
    let label = bottom
        .find("[tok cost")
        .expect("tok cost label present at width 40");
    let arrow = bottom
        .find('\u{2b07}')
        .expect("follow/jump arrow present on the same row");
    assert!(
        label < arrow,
        "left corner label must not overlap the right indicator; got: {bottom}"
    );
}

#[test]
fn tok_cost_hidden_in_copy_mode() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hi".into()));
    v.apply(&SessionEvent::LlmUsage {
        total_tokens: 2_000_000,
        input_tokens: 1_800_000,
        output_tokens: 200_000,
    });
    v.apply(&SessionEvent::Done);
    let buf = draw(&v, 60, 10, true, true, 0);
    let text = full_text(&buf);
    assert!(
        !text.contains("tok cost"),
        "copy mode renders clean text only; got: {text}"
    );
}

#[test]
fn tok_cost_dropped_on_narrow_width() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hi".into()));
    v.apply(&SessionEvent::Done);
    // 24 cols: the label plus the right-indicator reservation no longer fit.
    let buf = draw(&v, 24, 10, false, false, 0);
    let text = full_text(&buf);
    assert!(
        !text.contains("tok cost"),
        "narrow terminal must drop the label instead of colliding; got: {text}"
    );
}

#[test]
fn tok_cost_border_appends_call_cost_segment_when_timing() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hi".into()));
    v.apply(&SessionEvent::Done);
    let buf = draw(&v, 60, 10, true, false, 42_000);
    let bottom = row_text(&buf, buf.area.bottom() - 1, buf.area.width);
    assert!(
        bottom.contains("[tok cost 0] · [call cost 42s]"),
        "active turn appends the turn-cost segment after tok cost; got: {bottom}"
    );
}

// ── [sub x/x] badge ────────────────────────────────────────────────────────
// Width math for the fixtures below: tok "[tok cost 0]" (11 chars + 2
// padding = 13), turn "[call cost 42s]" (15), sub "[sub 1/3]" (9);
// avail = width - 2 (borders) - 12 (right-edge reservation).

#[test]
fn subtask_badge_hidden_when_total_zero() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hi".into()));
    v.apply(&SessionEvent::Done);
    let buf = draw(&v, 60, 10, true, false, 42_000);
    let bottom = row_text(&buf, buf.area.bottom() - 1, buf.area.width);
    assert!(
        bottom.contains("[tok cost 0] · [call cost 42s]") && !bottom.contains("[sub"),
        "no registered subtasks must render no badge; got: {bottom}"
    );
}

#[test]
fn subtask_badge_shows_completed_ratio_after_call_cost() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hi".into()));
    v.apply(&SessionEvent::Done);
    v.subtask_done = 1;
    v.subtask_total = 3;
    // Width 60 → avail 46 ≥ 13+15+9: all three segments fit in order.
    let buf = draw(&v, 60, 10, true, false, 42_000);
    let bottom = row_text(&buf, buf.area.bottom() - 1, buf.area.width);
    assert!(
        bottom.contains("[tok cost 0] · [call cost 42s] · [sub 1/3]"),
        "wide border keeps tok, call cost and the sub badge; got: {bottom}"
    );
}

#[test]
fn subtask_badge_drops_first_but_call_cost_remains() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hi".into()));
    v.apply(&SessionEvent::Done);
    v.subtask_done = 1;
    v.subtask_total = 3;
    // Width 46 → avail 32: tok+call (28) fit, the sub addition (9) does not.
    let buf = draw(&v, 46, 10, true, false, 42_000);
    let bottom = row_text(&buf, buf.area.bottom() - 1, buf.area.width);
    assert!(
        bottom.contains("[tok cost 0] · [call cost 42s]") && !bottom.contains("[sub"),
        "tight width drops the sub badge but keeps call cost; got: {bottom}"
    );
}

#[test]
fn subtask_tight_width_still_keeps_tok_when_call_cost_drops() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hi".into()));
    v.apply(&SessionEvent::Done);
    v.subtask_done = 2;
    v.subtask_total = 5;
    // Width 40 → avail 26: tok (13) fits, the call addition (15) does not
    // (and the sub badge stays dropped with it).
    let buf = draw(&v, 40, 10, true, false, 42_000);
    let bottom = row_text(&buf, buf.area.bottom() - 1, buf.area.width);
    assert!(
        bottom.contains("[tok cost 0]")
            && !bottom.contains("call cost 42s")
            && !bottom.contains("[sub"),
        "narrower width keeps only the tok segment; got: {bottom}"
    );
}

#[test]
fn tok_cost_border_drops_turn_segment_before_tok_on_narrow_width() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hi".into()));
    v.apply(&SessionEvent::Done);
    // 40 cols: the combined label overflows the reserved right-edge space, so
    // the turn segment drops first and the tok segment stays.
    let buf = draw(&v, 40, 10, false, false, 42_000);
    let bottom = row_text(&buf, buf.area.bottom() - 1, buf.area.width);
    assert!(
        bottom.contains("[tok cost 0]") && !bottom.contains("call cost 42s"),
        "graded dropping keeps tok, drops turn; got: {bottom}"
    );
}

/// The bottom-border corners keep a colour hierarchy: the
/// `[tok cost]`/`[call cost]` labels render in the warn colour — the same
/// as the status bar's task timer and running-spinner spans — the `·`
/// separator is muted, and the live `跟随中…` follow indicator keeps the
/// accent.
#[test]
fn tok_cost_corner_quiet_and_follow_indicator_accent() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hi".into()));
    v.apply(&SessionEvent::Done);
    let buf = draw(&v, 60, 10, true, false, 42_000);
    let y = buf.area.bottom() - 1;

    // Byte indices lie (multi-byte border glyphs), so scan cells directly.
    let warn_style = ratatui::style::Style::default().fg(theme::warn_color());
    let mut tok_fg = None;
    let mut sep_fg = None;
    let mut turn_fg = None;
    let mut follow_fg = None;
    let mut seen_tok_label = false;
    for x in 0..buf.area.width {
        let Some(cell) = buf.cell((x, y)) else {
            continue;
        };
        let sym = cell.symbol();
        if sym == "[" && tok_fg.is_none() {
            tok_fg = Some((cell.fg, cell.modifier));
        } else if sym == "t" && seen_tok_label {
            // First `t` after the tok label closes belongs to [call cost ...
            turn_fg = Some((cell.fg, cell.modifier));
            seen_tok_label = false;
        } else if sym == "]" && tok_fg.is_some() && turn_fg.is_none() {
            seen_tok_label = true;
        } else if sym == "\u{00b7}" && tok_fg.is_some() && sep_fg.is_none() {
            sep_fg = Some(cell.fg);
        } else if sym == "跟" {
            follow_fg = Some(cell.fg);
        }
    }
    assert_eq!(
        tok_fg,
        Some((warn_style.fg.unwrap(), warn_style.add_modifier)),
        "tok cost label must match the task-timer warn colour"
    );
    assert_eq!(
        turn_fg,
        Some((warn_style.fg.unwrap(), warn_style.add_modifier)),
        "call cost label must match the task-timer warn colour"
    );
    assert_eq!(sep_fg, Some(theme::muted()), "separator must be muted");
    assert_eq!(
        follow_fg,
        Some(theme::accent()),
        "follow indicator must keep the accent"
    );
}

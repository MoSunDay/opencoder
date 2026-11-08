//! The TURN LIVE REGION renders as the transcript frame's bottom strip:
//! BELOW the Say/ladder rows, ABOVE the `[tok cost]` bottom-border row —
//! and never inside the transcript's own line flow (flatten stays clean).
//! The strip HUGS the transcript's last rendered row (no blank gap when
//! the transcript is shorter than the frame) and is never truncated by a
//! height cap: the ladder-identical step rows render in full unless the
//! frame itself overflows.

use super::*;
use crate::chat::ChatView;
use opencoder_session::SessionEvent;
use ratatui::Terminal;

fn row_of(buf: &ratatui::buffer::Buffer, needle: &str, w: u16) -> Option<u16> {
    (0..buf.area.height).find(|&y| row_text(buf, y, w).contains(needle))
}

#[test]
fn live_strip_sits_below_say_rows_and_above_tok_cost_border() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("pondering".into()));
    v.apply(&SessionEvent::ToolStart {
        id: "c1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });

    // 24 rows: with no cap the strip renders its FULL 5 rows (header +
    // thinking header/body + the calls aggregate row + the call's header
    // row) directly below the transcript's last content row — only
    // physical frame overflow would clip (tail-kept).
    let backend = TestBackend::new(40, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let live_lines = crate::render_live::region(&v, false, 0);
    assert!(!live_lines.is_empty(), "a running turn arms the strip");
    let mut scroll = 0u32;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                &v,
                &Line::raw("test"),
                &mut scroll,
                false,
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
                0,
                false,
                None,
                &live_lines,
            );
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let w = buf.area.width;
    let ladder = row_of(buf, "Step", w).expect("folded ladder row renders");
    let live = row_of(buf, "live", w).expect("live strip renders");
    let tok = row_of(buf, "tok", w).expect("[tok cost] border renders");
    assert!(ladder < live, "strip sits BELOW the Say/ladder rows");
    assert!(live < tok, "strip sits ABOVE the [tok cost] border");
    // R1 hug: the strip starts right below the transcript's rendered rows.
    // Those end on the ladder's OWN trailing blank (a transcript contract,
    // `chat_flatten.rs`) with the group row directly above it — so the row
    // directly above `⏺ live` is that blank and the row above IT is the
    // ladder row: NOTHING is inserted between transcript and strip.
    let above = row_text(buf, live - 1, w);
    assert!(
        above.chars().all(|c| c.is_whitespace() || c == '\u{2502}'),
        "the row above the strip is the ladder's own trailing blank (not an \
inserted gap): {above:?}"
    );
    assert_eq!(
        live - 2,
        ladder,
        "the strip hugs the transcript: ladder row, its trailing blank, strip"
    );
    // The strip's call rows are LADDER-IDENTICAL (`push_step_details`,
    // calls forced open, results never): the aggregate row plus the call's
    // header row (name + one-line summary).
    assert!(
        row_of(buf, "Function call", w).is_some(),
        "the strip carries the calls aggregate row (ladder-identical shape)"
    );
    assert!(
        row_of(buf, "bash", w).is_some(),
        "the strip carries the running call's header row"
    );
    let header = row_of(buf, "echo x", w).expect("the strip carries the call header summary");
    let header_row = row_text(buf, header, w);
    assert!(
        header_row.contains("bash") && header_row.contains("echo x"),
        "the header row renders name + one-line summary: {header_row:?}"
    );
    // Render-only: the transcript's own flatten never carries strip rows.
    let flat: Vec<String> = v
        .flatten()
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect();
    assert!(
        !flat.iter().any(|l| l.contains("pondering")),
        "strip stays out of the transcript lines: {flat:?}"
    );
}

#[test]
fn streaming_say_renders_in_the_strip_until_sealed() {
    // The OPEN Say is TRANSCRIPT content now: it renders in its FINAL
    // shape while streaming — the merged `❯ Say(1 step)` group row forms
    // IMMEDIATELY with the running hint, its RAW body rows streaming below
    // — and the live region holds NO Say at all (it cleared the moment
    // the Say began: progress disarmed). Sealing only drops the hint.
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("pondering".into()));
    v.apply(&SessionEvent::ToolStart {
        id: "c1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    v.apply(&SessionEvent::TextDelta("the answer\nand more".into()));

    let live_lines = crate::render_live::region(&v, false, 0);
    let backend = TestBackend::new(40, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut scroll = 0u32;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                &v,
                &Line::raw("test"),
                &mut scroll,
                false,
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
                0,
                false,
                None,
                &live_lines,
            );
        })
        .unwrap();

    let buf = terminal.backend().buffer();
    let w = buf.area.width;
    // The strip carries NO Say marker and no say text — only the step
    // rows would render there, and the Say disarmed progress, so it is
    // empty entirely.
    assert!(
        live_lines.is_empty(),
        "the live region cleared when the Say began: {live_lines:?}"
    );
    assert!(
        row_of(buf, "\u{1f4ac}", w).is_none(),
        "no Say marker anywhere in the frame"
    );
    // The TRANSCRIPT carries the merged group row with the running hint,
    // and the RAW body rows stream below it (first line deduped into the
    // header preview).
    let merged = row_of(buf, "Say(1 step): the answer", w)
        .expect("the open Say merges onto the group row immediately");
    let running = row_of(buf, "running", w).expect("the merged row carries the running hint");
    assert_eq!(merged, running, "the hint rides the merged header row");
    let body = row_of(buf, "and more", w).expect("the raw body rows stream below");
    assert!(merged < body, "the Say body follows its merged header");
    let tok = row_of(buf, "tok", w).expect("[tok cost] border renders");
    assert!(body < tok, "the streaming Say scrolls inside the frame");

    // Seal: the shape is IDENTICAL minus the running hint.
    v.apply(&SessionEvent::Done);
    let live_lines = crate::render_live::region(&v, false, 0);
    assert!(live_lines.is_empty(), "the strip stays empty once sealed");
    let backend = TestBackend::new(40, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut scroll = 0u32;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                &v,
                &Line::raw("test"),
                &mut scroll,
                false,
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
                0,
                false,
                None,
                &live_lines,
            );
        })
        .unwrap();
    let buf = terminal.backend().buffer();
    let w = buf.area.width;
    let merged =
        row_of(buf, "Say(1 step): the answer", w).expect("the sealed pair keeps the merged header");
    let body = row_of(buf, "and more", w).expect("the sealed body keeps its rows");
    assert!(merged < body, "the body still follows the merged header");
    assert!(
        row_of(buf, "running", w).is_none(),
        "sealing drops the running hint — the only visual delta"
    );
}

/// The live content can grow past the frame (untruncated thinking + call
/// rows — steps only, the Say never rides the strip). The strip must then
/// take the WHOLE inner area, tail-kept — never blank the body (the
/// content_h == 0 regression).
#[test]
fn huge_live_takes_the_whole_frame_tail_kept_instead_of_blanking() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::QueueConsumed {
        seq: 1,
        text: "go".into(),
    });
    v.apply(&SessionEvent::ReasoningDelta(format!(
        "{}{}",
        "think line\n".repeat(18),
        "a very long thinking line ".repeat(4),
    )));
    v.apply(&SessionEvent::ToolStart {
        id: "c1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    let live_lines = crate::render_live::region(&v, false, 0);
    assert!(
        live_lines.len() > 10,
        "fixture: live exceeds the 10-row inner area"
    );
    let backend = TestBackend::new(60, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut scroll = 0u32;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                &v,
                &Line::raw("test"),
                &mut scroll,
                true,
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
                0,
                false,
                None,
                &live_lines,
            );
        })
        .unwrap();
    let buf = terminal.backend().buffer();
    // The inner area is NOT blank: the tail-kept strip fills it — the
    // NEWEST rows (the running call's header) visible, above the
    // `[tok cost]` border.
    let call = row_of(buf, "bash", 60).expect("running call header visible");
    let tok = row_of(buf, "tok", 60).expect("tok cost border");
    assert!(call < tok, "call row {call} above tok border {tok}");
    for y in 1..11u16 {
        assert!(
            !row_text(buf, y, 60).trim().is_empty(),
            "row {y} must not be blank while live overflows the frame"
        );
    }
}

/// Regression: the strip must stay VISIBLE when the transcript overflows
/// the frame. `strip_rect` used to receive TOTAL flattened rows: once
/// `total_rows >= inner.height` the strip's y landed on the frame bottom,
/// avail became 0 and the strip vanished. It must receive the VISIBLE
/// content rows instead, so the strip hugs the bottom of the visible
/// window — above the `[tok cost]` border.
#[test]
fn live_strip_stays_visible_when_transcript_overflows() {
    let mut v = ChatView::default();
    for i in 0..12 {
        v.blocks.push(ChatBlock::User {
            rendered: vec![Line::from(format!("user line {i}"))],
        });
    }
    v.apply(&SessionEvent::ReasoningDelta("pondering".into()));
    v.apply(&SessionEvent::ToolStart {
        id: "c1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    let live_lines = crate::render_live::region(&v, false, 0);
    assert!(!live_lines.is_empty(), "a running turn arms the strip");

    let backend = TestBackend::new(40, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut scroll = 0u32;
    terminal
        .draw(|f| {
            render_body(
                f,
                f.area(),
                &v,
                &Line::raw("test"),
                &mut scroll,
                true,
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
                0,
                false,
                None,
                &live_lines,
            );
        })
        .unwrap();
    let buf = terminal.backend().buffer();
    let w = buf.area.width;
    // The strip renders at all — the exact rows the old bug erased.
    let live = row_of(buf, "live", w).expect("the live strip stays visible");
    let call = row_of(buf, "bash", w).expect("the strip's call header renders");
    let tok = row_of(buf, "tok", w).expect("[tok cost] border renders");
    assert!(live < tok, "strip header {live} above tok border {tok}");
    assert!(call < tok, "strip call row {call} above tok border {tok}");
    assert!(call > live, "the call rows sit below the strip header");
    // The transcript above the strip is the scrolled content window.
    let user = row_of(buf, "user line", w).expect("transcript rows fill the frame");
    assert!(user < live, "transcript {user} above the strip {live}");
}

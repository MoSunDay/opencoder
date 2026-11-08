//! The TURN LIVE REGION rides the TAIL of the transcript's scroll window:
//! BELOW the Say/ladder rows, ABOVE the `[tok cost]` bottom-border row —
//! one scrollbar shared with the transcript (`render.rs::render_body`),
//! never truncated (scrolling up reveals the region's start). It is never
//! inside the transcript's own line flow (flatten stays clean): the
//! ladder-identical step rows render in full unless the frame itself
//! overflows (tail-kept).

use super::*;
use crate::chat::ChatView;
use opencoder_session::SessionEvent;
use ratatui::Terminal;

fn row_of(buf: &ratatui::buffer::Buffer, needle: &str, w: u16) -> Option<u16> {
    (0..buf.area.height).find(|&y| row_text(buf, y, w).contains(needle))
}

/// The unified scrollbar occupies the inner area's rightmost column; its
/// track/thumb are blank cells with a non-default background, so a painted
/// column proves the scrollbar was drawn.
fn scrollbar_painted(buf: &ratatui::buffer::Buffer, area: ratatui::layout::Rect) -> bool {
    let x = area.right().saturating_sub(2); // inner.right()-1
    (area.y + 1..area.bottom().saturating_sub(1)).any(|y| {
        buf.cell((x, y))
            .is_some_and(|c| c.bg != ratatui::style::Color::Reset)
    })
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
fn streaming_say_renders_standalone_in_the_transcript_until_sealed() {
    // The OPEN Say is TRANSCRIPT content streaming STANDALONE: while it
    // runs the ladder above stays FOLDED in its settled counting shape
    // (`▸ 1 Step`, NO merged `Say(n steps)` row), the `say:` header
    // carries the running spinner, and the RAW body rows render below it
    // in full (no preview dedup) — and the live region holds NO Say at
    // all (it cleared the moment the Say began: progress disarmed).
    // Sealing merges the pair and dedups the body against the preview.
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
    // The live region carries NO Say marker and no say text — only the
    // step rows would render there, and the Say disarmed progress, so it
    // is empty entirely.
    assert!(
        live_lines.is_empty(),
        "the live region cleared when the Say began: {live_lines:?}"
    );
    assert!(
        row_of(buf, "\u{1f4ac}", w).is_none(),
        "no Say marker anywhere in the frame"
    );
    // The TRANSCRIPT carries the folded ladder (NO merged Say row yet)…
    let group = row_of(buf, "1 Step", w).expect("the folded group row renders");
    let group_row = row_text(buf, group, w);
    assert!(
        !group_row.contains("Say("),
        "no merge while the Say streams — settled counting shape: {group_row:?}"
    );
    // …then the standalone `say:` header with the running spinner…
    let say = row_of(buf, "Say:", w).expect("the standalone say: header renders");
    let say_row = row_text(buf, say, w);
    assert!(
        say_row.contains("\u{280b}"),
        "the say: header carries the running spinner: {say_row:?}"
    );
    // …and the FULL raw body below it (no preview dedup while streaming).
    let first = row_of(buf, "the answer", w).expect("the raw first body row renders");
    let rest = row_of(buf, "and more", w).expect("the raw second body row renders");
    assert!(
        group < say && say < first && first < rest,
        "folded row, say: \
header, raw body — in order: {group} {say} {first} {rest}"
    );
    let tok = row_of(buf, "tok", w).expect("[tok cost] border renders");
    assert!(rest < tok, "the streaming Say scrolls inside the frame");

    // Seal: the pair MERGES — folded row + say: header collapse into the
    // `Say(n steps)` header and the body dedups its preview line away.
    v.apply(&SessionEvent::Done);
    let live_lines = crate::render_live::region(&v, false, 0);
    assert!(live_lines.is_empty(), "the region stays empty once sealed");
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
        row_of(buf, "running", w).is_none() && row_of(buf, "\u{280b}", w).is_none(),
        "sealing drops the spinner — the only visual delta"
    );
}

/// The live content can grow past the frame on its own (untruncated
/// thinking + call rows). The unified scroll window must then keep the
/// TAIL while following (newest rows visible, a scrollbar drawn) AND stay
/// scrollable to the region's START — scroll=0 reveals `⬥ live` and the
/// first thinking rows: never truncated, never blanked.
#[test]
fn huge_live_rides_the_scroll_window_tail_kept_and_start_reachable() {
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
    let area = terminal.get_frame().area();
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
    // Following pins the window to the TAIL: the newest rows (the running
    // call's header) visible, above the `[tok cost]` border — and the
    // unified scrollbar is drawn because content+live overflow the frame.
    let call = row_of(buf, "bash", 60).expect("running call header visible");
    let tok = row_of(buf, "tok", 60).expect("tok cost border");
    assert!(call < tok, "call row {call} above tok border {tok}");
    assert!(
        scrollbar_painted(buf, area),
        "content + live overflow the frame — one scrollbar for the window"
    );
    assert!(
        row_of(buf, "live", 60).is_none(),
        "the live START is scrolled off while pinned to the tail"
    );

    // Scroll all the way up: the window's very first rows are the
    // transcript head, then the live region's START — `⬥ live` and the
    // first thinking rows render un-truncated.
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
    let live = row_of(buf, "live", 60).expect("scroll=0 reveals the live start");
    let think = row_of(buf, "think line", 60).expect("the first thinking rows render");
    let tok = row_of(buf, "tok", 60).expect("tok cost border");
    assert!(
        live < think && think < tok,
        "live start, thinking rows in order"
    );
}

/// Regression: the live region must stay VISIBLE when the transcript
/// overflows the frame. The tail of the unified scroll window carries it:
/// following pins the window to the bottom, so the live rows always land
/// in view — hugging the frame's tail, above the `[tok cost]` border.
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

/// ONE scrollbar for content + live: when the transcript and the live
/// tail TOGETHER overflow the frame, the unified window draws a scrollbar
/// sized over `total_rows + live_rows`, and following shows the live tail
/// rows above the `[tok cost]` border.
#[test]
fn unified_scrollbar_covers_content_and_live_tail() {
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

    let backend = TestBackend::new(40, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut scroll = 0u32;
    let area = terminal.get_frame().area();
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
    // One scrollbar over the unified window: the transcript alone fills
    // the 10-row inner area, the live tail extends it further — without
    // counting the live rows no scrollbar (or a wrong-sized one) drew.
    assert!(
        scrollbar_painted(buf, area),
        "content + live overflow → scrollbar drawn over the unified window"
    );
    // Following shows the live TAIL rows, hugging the frame bottom —
    // above the `[tok cost]` border.
    let live = row_of(buf, "live", w).expect("the live header stays visible");
    let call = row_of(buf, "bash", w).expect("the live call header renders");
    let tok = row_of(buf, "tok", w).expect("[tok cost] border renders");
    assert!(
        live < call && call < tok,
        "live rows ride the window tail: {live} {call} {tok}"
    );
    // The window above the live tail is the transcript's OLDEST-scrolled
    // rows: following pins the unified window to its very bottom.
    let user = row_of(buf, "user line", w).expect("transcript rows fill the rest");
    assert!(user < live, "transcript {user} above the live tail {live}");
}

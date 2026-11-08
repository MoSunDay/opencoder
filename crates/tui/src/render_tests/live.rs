//! The TURN LIVE REGION renders as the transcript frame's bottom strip:
//! BELOW the Say/ladder rows, ABOVE the `[tok cost]` bottom-border row —
//! and never inside the transcript's own line flow (flatten stays clean).
//! The strip HUGS the transcript's last rendered row (no blank gap when
//! the transcript is shorter than the frame) and is never truncated by a
//! height cap: the expanded call input renders in full unless the frame
//! itself overflows.

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

    // 24 rows: with no cap the strip renders its FULL 7 rows (header +
    // thinking header/body + name row + 3 input lines) directly below the
    // transcript's last content row — only physical frame overflow would
    // clip (tail-kept).
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
    // The call's name row + EXPANDED input stream in the strip — the
    // input line present IN FULL (no one-third cap truncates it).
    assert!(
        row_of(buf, "bash", w).is_some(),
        "the strip carries the running call's name row"
    );
    let input = row_of(buf, "echo x", w).expect("the strip carries the input EXPANDED");
    let input_row = row_text(buf, input, w);
    assert!(
        input_row.contains("\"command\": \"echo x\""),
        "the expanded input line is not truncated: {input_row:?}"
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
    // R2: an OPEN Say is not transcript content — the strip carries its
    // `💬 Say` marker + raw text right below the ladder, while the
    // transcript shows the ladder STANDALONE (no merged `Say(n steps)`
    // header until the Say seals).
    let mut v = ChatView::default();
    v.apply(&SessionEvent::ReasoningDelta("pondering".into()));
    v.apply(&SessionEvent::ToolStart {
        id: "c1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    v.apply(&SessionEvent::TextDelta("the answer".into()));

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
    let say = row_of(buf, "\u{1f4ac}", w).expect("the strip shows the Say marker");
    let answer = row_of(buf, "the answer", w).expect("the strip shows the streamed text");
    let ladder = row_of(buf, "1 Step", w).expect("the ladder renders standalone");
    assert!(ladder < say, "the Say marker sits below the ladder: {say}");
    assert!(say < answer, "the Say text follows its marker: {answer}");
    let above = row_text(buf, say - 1, w);
    assert!(
        !above.trim().is_empty(),
        "no blank gap above the strip while the Say streams: {above:?}"
    );

    // The transcript holds NO say rows and no merged header while open.
    let flat: Vec<String> = v
        .flatten()
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect();
    assert!(
        !flat
            .iter()
            .any(|l| l.contains("Say(") || l.contains("the answer")),
        "an open Say is not transcript content: {flat:?}"
    );

    // Seal: the strip clears and the transcript gains the merged header.
    v.apply(&SessionEvent::Done);
    let live_lines = crate::render_live::region(&v, false, 0);
    assert!(live_lines.is_empty(), "the strip clears once sealed");
    let flat: Vec<String> = v
        .flatten()
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
        .collect();
    assert!(
        flat.iter().any(|l| l.contains("Say(1 step): the answer")),
        "the sealed Say lands in the transcript as the merged header: {flat:?}"
    );
}

/// The live content can grow past the frame (untruncated thinking + input
/// + streaming Say). The strip must then take the WHOLE inner area,
/// tail-kept — never blank the body (the content_h == 0 regression).
#[test]
fn huge_live_takes_the_whole_frame_tail_kept_instead_of_blanking() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::QueueConsumed {
        seq: 1,
        text: "go".into(),
    });
    v.apply(&SessionEvent::ReasoningDelta(
        "think line\n".repeat(20).into(),
    ));
    v.apply(&SessionEvent::ToolStart {
        id: "c1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "echo x"}),
    });
    v.apply(&SessionEvent::TextDelta(
        "say text streaming along\n".repeat(5).into(),
    ));
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
    // The inner area is NOT blank: the tail-kept strip fills it, newest
    // rows (the streaming Say) visible, above the `[tok cost]` border.
    let say = row_of(buf, "say text", 60).expect("streaming Say visible");
    let tok = row_of(buf, "tok", 60).expect("tok cost border");
    assert!(say < tok, "say row {say} above tok border {tok}");
    for y in 1..11u16 {
        assert!(
            !row_text(buf, y, 60).trim().is_empty(),
            "row {y} must not be blank while live overflows the frame"
        );
    }
}

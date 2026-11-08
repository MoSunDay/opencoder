//! The TURN LIVE REGION renders as the transcript frame's bottom strip:
//! BELOW the Say/ladder rows, ABOVE the `[tok cost]` bottom-border row —
//! and never inside the transcript's own line flow (flatten stays clean).

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

    // 24 rows: the one-third budget (8) fits the full strip incl. header —
    // the expanded input grows the strip to 7 rows (header + thinking
    // header/body + name row + 3 input lines); smaller frames tail-keep the
    // newest rows and may drop the marker.
    let backend = TestBackend::new(40, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let (live_lines, live_h) = crate::render_live::region(&v, false, 0, 24);
    assert!(live_h > 0, "a running turn arms the strip");
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
                live_h,
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
    // The call's name row + EXPANDED input stream in the strip.
    assert!(
        row_of(buf, "bash", w).is_some(),
        "the strip carries the running call's name row"
    );
    assert!(
        row_of(buf, "echo x", w).is_some(),
        "the strip carries the call's input EXPANDED"
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

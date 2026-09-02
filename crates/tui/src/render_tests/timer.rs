use super::*;
use crate::chat::ChatView;
use opencoder_session::SessionEvent;
use ratatui::backend::TestBackend;

fn render_body_with_tail(content: &str, tail_ms: u64, width: u16, height: u16) -> String {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta(content.into()));
    v.apply(&SessionEvent::Done);
    let backend = TestBackend::new(width, height);
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
                &mut Vec::new(),
                &mut None,
                true,
                tail_ms,
                false,
                None,
            );
        })
        .unwrap();
    (0..height)
        .map(|y| row_text(terminal.backend().buffer(), y, width))
        .collect::<Vec<_>>()
        .join("\n")
}

/// When tail_ms > 0, the whole-turn timer (`[call cost ...]`) renders on the
/// body block's bottom border, appended after `[tok cost]` with a `·` —
/// the in-body tail row was removed when the timer moved to the corner.
#[test]
fn body_shows_call_cost_timer_at_content_tail() {
    let full = render_body_with_tail("hello world", 42000, 60, 8);
    assert!(
        full.contains("[call cost 42s]"),
        "body should show call cost timer at content tail; got:\n{full}"
    );
    assert!(
        full.contains("[tok cost 0] · [call cost 42s]"),
        "the timer must ride the bottom-border corner after tok cost; got:\n{full}"
    );
}

/// When tail_ms is 0, no timer is shown.
#[test]
fn body_hides_call_cost_timer_when_zero() {
    let full = render_body_with_tail("hello world", 0, 60, 8);
    assert!(
        !full.contains("[call cost"),
        "zero tail timer should not render; got:\n{full}"
    );
}

/// The turn-cost timer never shares a row with content text (its only home
/// is the border corner row). This prevents it from blending into bash/tool
/// output lines at the transcript tail.
#[test]
fn body_call_cost_timer_on_own_line() {
    let full = render_body_with_tail("hello world", 42000, 60, 8);
    let mut found_content = false;
    let mut found_timer = false;
    for row in full.lines() {
        let trimmed = row.trim().trim_matches('\u{2502}').trim();
        if trimmed.contains("hello world") && !trimmed.contains("[call cost") {
            found_content = true;
        }
        if trimmed.contains("[call cost 42s]") {
            assert!(
                !trimmed.contains("hello world"),
                "timer must be on its own line, not sharing with content; got: {row}"
            );
            found_timer = true;
        }
    }
    assert!(
        found_content,
        "content row not found in body output:\n{full}"
    );
    assert!(found_timer, "timer not found in body output:\n{full}");
}

/// Regardless of content width, the timer never blends into a wrapped content
/// row: its only home is the bottom-border corner row, which it shares solely
/// with the `[tok cost]` segment and border decoration.
#[test]
fn body_call_cost_timer_always_own_line() {
    // 50 chars still wrap inside a 60-col body (inner ~56); 60 keeps both
    // corner segments on the border row (narrower widths drop the turn
    // segment by the graded guard — covered in tok_cost render tests).
    let content = "abcdefghij".repeat(5);
    let full = render_body_with_tail(&content, 42000, 60, 8);
    let mut found_timer = false;
    for row in full.lines() {
        if row.contains("[call cost 42s]") {
            assert!(
                row.contains("[tok cost"),
                "timer must ride the corner row next to tok cost; got: {row}"
            );
            assert!(
                !row.contains("abcdefghij"),
                "timer must never share a row with wrapped content; got: {row}"
            );
            found_timer = true;
        }
    }
    assert!(found_timer, "timer not found in body output:\n{full}");
}

/// Regression: when the transcript tail is a bash/tool-output block (expanded),
/// the `[call cost]` timer must NOT be appended onto the tool output line.
/// It must appear on its own dedicated line so the duration is never visually
/// folded into truncated tool output. (Issue: timer blends into bash output.)
#[test]
fn body_call_cost_timer_not_mixed_into_tool_output() {
    use serde_json::json;

    let mut v = ChatView::default();
    // A bash tool block that ends the transcript with output lines.
    v.apply(&SessionEvent::ToolStart {
        id: "t1".into(),
        name: "bash".into(),
        input: json!("ls -la"),
    });
    v.apply(&SessionEvent::ToolEnd {
        id: "t1".into(),
        name: "bash".into(),
        output: "line one of bash output\nline two of bash output".into(),
        is_error: false,
        images: vec![],
    });
    // Cycle the tool group to Results so its output is visible (collapsed
    // hides output behind the group line, but the group line is itself a
    // content line that the old code would append the timer to).
    v.cycle_tool_group_at(0);
    v.cycle_tool_group_at(0);
    v.apply(&SessionEvent::Done);

    let backend = TestBackend::new(60, 10);
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
                &mut Vec::new(),
                &mut None,
                true,
                30000,
                false,
                None,
            );
        })
        .unwrap();

    let full: String = (0..10)
        .map(|y| row_text(terminal.backend().buffer(), y, 60))
        .collect::<Vec<_>>()
        .join("\n");

    // The timer must exist.
    assert!(
        full.contains("[call cost"),
        "timer should be visible; got:\n{full}"
    );
    // No single row may contain BOTH bash output content AND the timer.
    for row in full.lines() {
        let has_bash_output = row.contains("line one") || row.contains("line two");
        let has_timer = row.contains("[call cost");
        assert!(
            !(has_bash_output && has_timer),
            "timer must not share a line with bash output; got: {row}"
        );
    }
}

/// The timer stays visible even when the viewport is scrolled away from the
/// tail (regression: the old `end == n` gate hid it whenever content
/// overflowed or the user scrolled up).
#[test]
fn body_timer_visible_when_scrolled_away_from_tail() {
    let mut v = ChatView::default();
    // Enough content to overflow a small viewport.
    for i in 0..20 {
        v.apply(&SessionEvent::TextDelta(format!(
            "content line number {}\n",
            i
        )));
    }
    v.apply(&SessionEvent::Done);

    let backend = TestBackend::new(60, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut scroll = 0u32; // pinned to top, NOT following
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
                &mut Vec::new(),
                &mut None,
                true,
                30000,
                false,
                None,
            );
        })
        .unwrap();

    let full: String = (0..8)
        .map(|y| row_text(terminal.backend().buffer(), y, 60))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        full.contains("[call cost"),
        "timer must be visible even when scrolled away from tail; got:\n{full}"
    );
}

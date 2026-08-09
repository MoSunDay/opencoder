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
                &mut None,
                true,
                tail_ms,
            );
        })
        .unwrap();
    (0..height)
        .map(|y| row_text(terminal.backend().buffer(), y, width))
        .collect::<Vec<_>>()
        .join("\n")
}

/// When tail_ms > 0, the body shows the whole-turn timer (`[turn cost ...]`) at
/// the tail of the last content line.
#[test]
fn body_shows_turn_cost_timer_at_content_tail() {
    let full = render_body_with_tail("hello world", 42000, 60, 8);
    assert!(
        full.contains("[turn cost 42s]"),
        "body should show turn cost timer at content tail; got:\n{full}"
    );
}

/// When tail_ms is 0, no timer is shown.
#[test]
fn body_hides_turn_cost_timer_when_zero() {
    let full = render_body_with_tail("hello world", 0, 60, 8);
    assert!(
        !full.contains("[turn cost"),
        "zero tail timer should not render; got:\n{full}"
    );
}

/// The turn-cost timer always occupies its own dedicated line, never sharing
/// a row with content text. This prevents it from blending into bash/tool
/// output lines at the transcript tail.
#[test]
fn body_turn_cost_timer_on_own_line() {
    let full = render_body_with_tail("hello world", 42000, 60, 8);
    let mut found_content = false;
    let mut found_timer = false;
    for row in full.lines() {
        let trimmed = row.trim().trim_matches('\u{2502}').trim();
        if trimmed.contains("hello world") && !trimmed.contains("[turn cost") {
            found_content = true;
        }
        if trimmed.contains("[turn cost 42s]") {
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

/// Regardless of content width, the timer always lands on a dedicated line.
#[test]
fn body_turn_cost_timer_always_own_line() {
    let content = "abcdefghij".repeat(5); // 50 chars — wider than viewport
    let full = render_body_with_tail(&content, 42000, 40, 8);
    assert!(
        full.lines()
            .any(|r| r.trim().trim_matches('\u{2502}').trim() == "[turn cost 42s]"),
        "timer must be on its own line regardless of content width; got:\n{full}"
    );
}

/// Regression: when the transcript tail is a bash/tool-output block (expanded),
/// the `[turn cost]` timer must NOT be appended onto the tool output line.
/// It must appear on its own dedicated line so the duration is never visually
/// folded into truncated tool output. (Issue: timer blends into bash output.)
#[test]
fn body_turn_cost_timer_not_mixed_into_tool_output() {
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
    // Expand the tool block so its output is visible (collapsed hides output
    // behind the header, but the header line is itself a content line that
    // the old code would append the timer to).
    v.toggle_tool_at(0);
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
                &mut None,
                true,
                30000,
            );
        })
        .unwrap();

    let full: String = (0..10)
        .map(|y| row_text(terminal.backend().buffer(), y, 60))
        .collect::<Vec<_>>()
        .join("\n");

    // The timer must exist.
    assert!(
        full.contains("[turn cost"),
        "timer should be visible; got:\n{full}"
    );
    // No single row may contain BOTH bash output content AND the timer.
    for row in full.lines() {
        let has_bash_output = row.contains("line one") || row.contains("line two");
        let has_timer = row.contains("[turn cost");
        assert!(
            !(has_bash_output && has_timer),
            "timer must not share a line with bash output; got: {row}"
        );
    }
}

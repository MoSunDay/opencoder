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
                None,
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

/// When tail_ms > 0, the body shows the call-round timer (`[call ...]`) at
/// the tail of the last content line.
#[test]
fn body_shows_call_timer_at_content_tail() {
    let full = render_body_with_tail("hello world", 42000, 60, 8);
    assert!(
        full.contains("[call 42s]"),
        "body should show call timer at content tail; got:\n{full}"
    );
}

/// When tail_ms is 0, no timer is shown.
#[test]
fn body_hides_call_timer_when_zero() {
    let full = render_body_with_tail("hello world", 0, 60, 8);
    assert!(
        !full.contains("[call"),
        "zero tail timer should not render; got:\n{full}"
    );
}

/// The call timer appears at the tail — after the content text on the same line.
#[test]
fn body_call_timer_after_content() {
    let full = render_body_with_tail("hello world", 42000, 60, 8);
    for row in full.lines() {
        if let Some(content_pos) = row.find("hello world") {
            let timer_pos = row.find("[call 42s]");
            assert!(
                timer_pos.is_some() && timer_pos > Some(content_pos),
                "timer must appear after content text on the same line; got: {row}"
            );
            return;
        }
    }
    panic!("content row not found in body output:\n{full}");
}

/// When the content line is too full to fit the timer, it must wrap onto its
/// own dedicated line — never be dropped.
#[test]
fn body_call_timer_wraps_to_own_line_when_full() {
    let content = "abcdefghij".repeat(5); // 50 chars > fits-with-timer budget
    let full = render_body_with_tail(&content, 42000, 40, 8);
    assert!(
        full.lines().any(|r| r.trim().trim_matches('\u{2502}').trim() == "[call 42s]"),
        "full content line must push timer to its own line; got:\n{full}"
    );
}

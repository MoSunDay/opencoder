use super::*;
use crate::chat::ChatView;
use opencoder_session::SessionEvent;
use ratatui::backend::TestBackend;

fn render_body_with_turn(turn_ms: u64, width: u16, height: u16) -> String {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("hello world".into()));
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
                "test",
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
                turn_ms,
            );
        })
        .unwrap();
    (0..height)
        .map(|y| row_text(terminal.backend().buffer(), y, width))
        .collect::<Vec<_>>()
        .join("\n")
}

/// When turn_ms > 0, the body shows the turn-duration timer at the tail of
/// the last content line.
#[test]
fn body_shows_turn_timer_at_content_tail() {
    let full = render_body_with_turn(42000, 60, 8);
    assert!(
        full.contains("42s"),
        "body should show turn timer at content tail; got:\n{full}"
    );
}

/// When turn_ms is 0, no timer is shown.
#[test]
fn body_hides_turn_timer_when_zero() {
    let full = render_body_with_turn(0, 60, 8);
    assert!(
        !full.contains("0s"),
        "zero turn timer should not render; got:\n{full}"
    );
}

/// The turn timer appears at the tail — after the content text on the same line.
#[test]
fn body_turn_timer_after_content() {
    let full = render_body_with_turn(42000, 60, 8);
    for row in full.lines() {
        if let Some(content_pos) = row.find("hello world") {
            let timer_pos = row.find("42s");
            assert!(
                timer_pos.is_some() && timer_pos > Some(content_pos),
                "timer must appear after content text on the same line; got: {row}"
            );
            return;
        }
    }
    panic!("content row not found in body output:\n{full}");
}

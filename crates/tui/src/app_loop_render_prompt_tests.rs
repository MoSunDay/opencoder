//! Tests for `input_event_prompts_frame`: the render-on-input contract that
//! decouples key-echo latency from the fps-configured frame ticker.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

use crate::app::app_loop::input_event_prompts_frame;

#[test]
fn all_input_surfaces_prompt_an_immediate_frame() {
    let cases: Vec<crossterm::event::Event> = vec![
        // Plain ASCII typing.
        crossterm::event::Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
        // CJK char delivered via IME commit (single Char event, multibyte).
        crossterm::event::Event::Key(KeyEvent::new(KeyCode::Char('中'), KeyModifiers::NONE)),
        // Bracketed paste (IME bulk commit / terminal paste).
        crossterm::event::Event::Paste("中文粘贴 multi-line\n".to_string()),
        // Mouse wheel over the transcript.
        crossterm::event::Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 3,
            row: 7,
            modifiers: KeyModifiers::NONE,
        }),
        // Terminal / tmux client resize.
        crossterm::event::Event::Resize(120, 40),
    ];
    for ev in &cases {
        assert!(input_event_prompts_frame(ev), "should prompt frame: {ev:?}");
    }
}

#[test]
fn focus_events_do_not_prompt_frames() {
    assert!(!input_event_prompts_frame(
        &crossterm::event::Event::FocusGained
    ));
    assert!(!input_event_prompts_frame(
        &crossterm::event::Event::FocusLost
    ));
}

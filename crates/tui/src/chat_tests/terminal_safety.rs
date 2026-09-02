use super::super::*;

fn flattened_text(view: &ChatView) -> String {
    view.flatten()
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect()
}

fn assert_no_terminal_controls(text: &str) {
    assert!(
        text.chars().all(|ch| !ch.is_control() || ch == '\n'),
        "terminal control leaked into flattened UI text: {text:?}"
    );
}

#[test]
fn thinking_delta_is_sanitized_once_before_it_reaches_rendering() {
    let mut view = ChatView::default();
    view.apply(&SessionEvent::ReasoningDelta(
        "old\rNEW\x08\x1b[2J\u{009b}31m\tline".into(),
    ));
    view.toggle_thinking_at(0);

    let text = flattened_text(&view);
    assert_no_terminal_controls(&text);
    assert!(text.contains("oldNEW[2J31m    line"), "got: {text:?}");
}

#[test]
fn every_dynamic_chat_block_uses_the_same_terminal_safety_boundary() {
    let dirty = "before\r\x1b[2J\x08\u{009b}after";
    let mut view = ChatView::default();
    view.apply(&SessionEvent::TextDelta(dirty.into()));
    view.apply(&SessionEvent::ToolStart {
        id: "tool".into(),
        name: dirty.into(),
        input: serde_json::json!({"command": dirty}),
    });
    view.apply(&SessionEvent::ToolEnd {
        id: "tool".into(),
        name: dirty.into(),
        output: dirty.into(),
        is_error: false,
        images: Vec::new(),
    });
    // Toggle the group twice (net closed): dirty content must survive the
    // toggle path untouched.
    view.toggle_step_group_at(1);
    view.toggle_step_group_at(1);
    view.apply(&SessionEvent::CompactionDelta(dirty.into()));
    view.apply(&SessionEvent::Status(dirty.into()));
    view.apply(&SessionEvent::Error(dirty.into()));

    assert_no_terminal_controls(&flattened_text(&view));
    assert_no_terminal_controls(&view.status);
}

use opencoder_session::SessionEvent;
use opencoder_tui::chat::{block_text, ChatView};

#[test]
fn retry_discards_partial_text_and_reasoning_but_preserves_previous_tool_results() {
    let mut view = ChatView::default();
    view.apply(&SessionEvent::ToolStart {
        id: "read".into(),
        name: "read".into(),
        input: serde_json::json!({"path":"a"}),
    });
    view.apply(&SessionEvent::ToolEnd {
        id: "read".into(),
        name: "read".into(),
        output: "saved contents".into(),
        is_error: false,
        images: vec![],
    });
    let before = view.blocks.clone();
    let context = view.context_used;
    view.apply(&SessionEvent::LlmRoundStart {
        started_at_ms: 1000,
    });
    view.apply(&SessionEvent::ReasoningDelta("discard thinking".into()));
    view.apply(&SessionEvent::TextDelta("discard text".into()));
    view.apply(&SessionEvent::LlmAttemptReset);
    assert_eq!(view.blocks, before);
    assert_eq!(view.context_used, context);
    view.apply(&SessionEvent::TextDelta("fresh answer".into()));
    view.apply(&SessionEvent::LlmRoundEnd);
    assert!(!block_text(&view).contains("discard"));
    assert!(block_text(&view).contains("fresh answer"));
    assert!(view.attempt_snapshot.is_none());
}

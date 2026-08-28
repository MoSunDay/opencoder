use super::super::*;

#[test]
fn agent_switch_updates_agent_without_marker() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::AgentSwitch("act".into()));
    assert_eq!(v.agent, "act");
    assert!(
        !v.blocks.iter().any(|b| matches!(b, ChatBlock::Marker(_))),
        "AgentSwitch must not pollute the chat body with a marker"
    );
}

#[test]
fn agent_switch_finalizes_pending_assistant() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("mid-stream".into()));
    v.apply(&SessionEvent::AgentSwitch("act".into()));
    let pending = v
        .blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::Assistant { done, .. } => Some(*done),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(!pending.is_empty(), "assistant block should exist");
    assert!(
        pending.iter().all(|d| *d),
        "assistant block must be finalized on AgentSwitch"
    );
}

/// Issue #1: the `[model]` chat marker must show the bare model id (no
/// `provider/` prefix), matching the status bar — both when the event carries
/// the full string (defensive strip) and the bare id (worker now emits bare).
#[test]
fn model_switch_marker_strips_provider_prefix() {
    let mut v = ChatView::default();

    // Full "provider/model" string -> rendered as bare id.
    v.apply(&SessionEvent::ModelSwitch("bigmodel/glm-5.2".into()));
    let text = block_text(&v);
    assert!(text.contains("[model]"), "marker prefix present");
    assert!(text.contains("glm-5.2"), "bare model id present");
    assert!(
        !text.contains('/'),
        "marker must not leak the provider slash: {text:?}"
    );

    // Bare id (what the worker now emits) -> unchanged, no slash.
    v.apply(&SessionEvent::ModelSwitch("glm-5.2".into()));
    let text2 = block_text(&v);
    assert!(text2.contains("glm-5.2"));
    assert!(!text2.contains('/'));
}

// ---------------------------------------------------------------------------
// P0: Tool-returned images render inline in the transcript (live path).
// When a ToolEnd carries `images`, each must produce a ChatBlock::Image.
// ---------------------------------------------------------------------------

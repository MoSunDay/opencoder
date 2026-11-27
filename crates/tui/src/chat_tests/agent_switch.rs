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

/// A model switch finalizes the pending assistant text and syncs the
/// live-session model the title/status bar render, but adds NO transcript
/// marker: the `[/model] ... switched` confirmation already covers the
/// switch, and a second `[model] <id>` line only duplicated it (it also
/// resurfaced on remote event replay). The model keeps whatever id the event
/// carries — the title strips a `provider/` prefix itself.
#[test]
fn model_switch_updates_model_without_marker() {
    let mut v = ChatView::default();
    v.apply(&SessionEvent::TextDelta("mid-stream".into()));

    // Full "provider/model" string -> model synced verbatim, no marker.
    v.apply(&SessionEvent::ModelSwitch("bigmodel/glm-5.2".into()));
    assert_eq!(
        v.model, "bigmodel/glm-5.2",
        "live model synced for the title"
    );
    assert!(
        !v.blocks.iter().any(|b| matches!(b, ChatBlock::Marker(_))),
        "ModelSwitch must not add a transcript marker; blocks: {:?}",
        v.blocks
    );
    let pending = v
        .blocks
        .iter()
        .filter_map(|b| match b {
            ChatBlock::Assistant { done, .. } => Some(*done),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        pending.iter().all(|d| *d),
        "assistant block must be finalized on ModelSwitch"
    );

    // Bare id (what the worker emits) -> same contract.
    v.apply(&SessionEvent::ModelSwitch("glm-5.2".into()));
    assert_eq!(v.model, "glm-5.2");
    assert!(
        !v.blocks.iter().any(|b| matches!(b, ChatBlock::Marker(_))),
        "still no marker after a second switch"
    );
    assert!(!block_text(&v).contains("[model]"));
}

// ---------------------------------------------------------------------------
// P0: Tool-returned images render inline in the transcript (live path).
// When a ToolEnd carries `images`, each must produce a ChatBlock::Image.
// ---------------------------------------------------------------------------

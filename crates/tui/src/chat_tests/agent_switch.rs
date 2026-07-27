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

#[test]
fn plan_submitted_defaults_false() {
    let v = ChatView::default();
    assert!(
        !v.plan_submitted,
        "plan_submitted must default to false so a fresh session never \
         triggers the plan->act handoff spuriously"
    );
}

#[test]
fn agent_switch_to_plan_resets_plan_submitted() {
    // Regression: switching into plan mode must reset the flag so that the
    // plan->act handoff only fires when the user actually submitted a prompt
    // during THIS plan session. Previously the check used
    // !chat.blocks.is_empty(), which is always true (blocks hold act history),
    // causing an accidental plan->act toggle to collapse the transcript.
    let mut v = ChatView {
        plan_submitted: true,
        ..Default::default()
    };
    v.apply(&SessionEvent::AgentSwitch("plan".into()));
    assert!(
        !v.plan_submitted,
        "entering plan mode must reset plan_submitted to false"
    );
}

#[test]
fn agent_switch_to_act_keeps_plan_submitted() {
    // Switching to act must NOT reset the flag — the app.rs event loop reads
    // it BEFORE the AgentSwitch event arrives to decide handoff vs plain swap.
    let mut v = ChatView {
        plan_submitted: true,
        ..Default::default()
    };
    v.apply(&SessionEvent::AgentSwitch("act".into()));
    assert!(
        v.plan_submitted,
        "switching to act must not clobber plan_submitted"
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


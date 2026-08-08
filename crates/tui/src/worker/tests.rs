use super::*;
use std::sync::Mutex;
use tokio::sync::mpsc;

fn test_session(id: &str) -> SessionState {
    SessionState::new(
        id,
        opencoder_core::resolve_agent("act").unwrap(),
        opencoder_core::Config::default(),
        Arc::new(opencoder_llm::MockChatClient::new()),
        std::env::temp_dir(),
    )
}

#[test]
fn gate_compact_runs_when_idle() {
    assert_eq!(gate_compact(false), CompactGate::Run);
}

#[test]
fn gate_compact_rejects_when_running() {
    assert_eq!(gate_compact(true), CompactGate::SkipRunning);
}

#[test]
fn gate_clear_all_runs_when_idle() {
    // Idle == all subagents returned → clear is allowed.
    assert_eq!(gate_clear_all(false), ClearAllGate::Run);
}

#[test]
fn gate_clear_all_rejects_when_running() {
    // A turn/subagent in flight must not be cleared (child session live).
    assert_eq!(gate_clear_all(true), ClearAllGate::SkipRunning);
}

#[test]
fn gate_switch_runs_when_idle() {
    // Idle: a clean turn boundary — the switch applies immediately.
    assert_eq!(gate_switch(false), SwitchGate::Run);
}

#[test]
fn gate_switch_rejects_when_running() {
    // A turn in flight: applying the mode switch now would start the next
    // turn with a stale agent at an arbitrary partial boundary.
    assert_eq!(gate_switch(true), SwitchGate::SkipRunning);
}

// F1 + G1 guard: after a `/task` switch, all parent and child runtime
// handles must point at the NEW session.
#[test]
fn rebind_session_swaps_the_active_cancel_token() {
    let (mut cmd_tx, _) = mpsc::channel::<UiCmd>(8);
    let (_, mut evt_rx) = mpsc::channel::<UiEvent>(8);
    let (new_cmd_tx, _) = mpsc::channel::<UiCmd>(8);
    let (_, new_evt_rx) = mpsc::channel::<UiEvent>(8);
    let mut session_id = String::from("s1");
    let first_cancel = CancellationToken::new();
    let mut cancel = first_cancel.clone();
    let mut turn_cancel: SharedCancel = Arc::new(Mutex::new(CancellationToken::new()));
    let new_cancel = CancellationToken::new();
    let new_cancel_probe = new_cancel.clone();
    let new_turn_cancel: SharedCancel = Arc::new(Mutex::new(CancellationToken::new()));
    let new_tc_probe = new_turn_cancel.lock().unwrap().clone();
    let old_session = test_session("old-runtime");
    let new_session = test_session("new-runtime");
    let mut child_runtime = ChildRuntimeHandles::from_session(&old_session);
    let old_child_cancels = child_runtime.cancels.clone();
    let old_child_turn_cancels = child_runtime.turn_cancels.clone();
    let old_child_steer_gates = child_runtime.steer_gates.clone();
    let new_child_runtime = ChildRuntimeHandles::from_session(&new_session);
    let new_child_cancels = new_child_runtime.cancels.clone();
    let new_child_turn_cancels = new_child_runtime.turn_cancels.clone();
    let new_child_steer_gates = new_child_runtime.steer_gates.clone();

    rebind_session(
        &mut cmd_tx,
        &mut evt_rx,
        &mut session_id,
        &mut cancel,
        &mut turn_cancel,
        &mut child_runtime,
        new_cmd_tx,
        new_evt_rx,
        "s2".into(),
        new_cancel,
        new_turn_cancel,
        new_child_runtime,
    );

    cancel.cancel();
    turn_cancel.lock().unwrap().cancel();
    assert!(
        new_cancel_probe.is_cancelled(),
        "active cancel targets switched session"
    );
    assert!(!first_cancel.is_cancelled(), "old session cancel orphaned");
    assert!(
        new_tc_probe.is_cancelled(),
        "turn_cancel targets switched session"
    );
    assert!(Arc::ptr_eq(&child_runtime.cancels, &new_child_cancels));
    assert!(!Arc::ptr_eq(&child_runtime.cancels, &old_child_cancels));
    assert!(Arc::ptr_eq(
        &child_runtime.turn_cancels,
        &new_child_turn_cancels
    ));
    assert!(!Arc::ptr_eq(
        &child_runtime.turn_cancels,
        &old_child_turn_cancels
    ));
    assert!(Arc::ptr_eq(
        &child_runtime.steer_gates,
        &new_child_steer_gates
    ));
    assert!(!Arc::ptr_eq(
        &child_runtime.steer_gates,
        &old_child_steer_gates
    ));
    assert_eq!(session_id, "s2");
}

// Regression guard for the "Esc then can't submit" bug: after a double-Esc
// abort the session's cancel token is permanently cancelled. The loop
// recovers by sending `ResetCancel(fresh)` before the next turn. This test
// verifies that `process_cmd(ResetCancel)` actually swaps `sess.cancel` for
// a fresh, uncancelled token — the exact invariant `run_loop` relies on at
// its top-of-loop `is_cancelled()` check.
#[tokio::test]
async fn reset_cancel_replaces_with_fresh_uncancelled_token() {
    use opencoder_core::resolve_agent;
    use opencoder_llm::MockChatClient;

    let (evt_tx, _evt_rx) = mpsc::channel::<UiEvent>(8);
    let agent = resolve_agent("act").expect("act agent");
    let stale = CancellationToken::new();
    stale.cancel();
    let stale_probe = stale.clone();
    let mut sess = SessionState::new(
        "reset-test",
        agent,
        opencoder_core::Config::default(),
        std::sync::Arc::new(MockChatClient::new()) as std::sync::Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    )
    .with_cancel(stale);
    assert!(
        sess.cancel.as_ref().unwrap().is_cancelled(),
        "precondition: token cancelled"
    );

    let fresh = CancellationToken::new();
    let fresh_probe = fresh.clone();
    let should_break = process_cmd(UiCmd::ResetCancel(fresh), &mut sess, &evt_tx).await;

    assert!(!should_break, "ResetCancel must not break the worker loop");
    let active = sess.cancel.as_ref().expect("token present after reset");
    assert!(
        !active.is_cancelled(),
        "session token must be uncancelled after reset"
    );
    assert!(
        !fresh_probe.is_cancelled(),
        "the fresh token itself must be uncancelled"
    );
    assert!(
        stale_probe.is_cancelled(),
        "the old token must remain cancelled (not reused)"
    );
}

// EditPlan rewrites the Text blocks of the last non-empty Assistant message
// in-memory while preserving non-Text blocks (Reasoning/ToolUse/etc.). This
// guards the plan-mode edit path: an edit that dropped Reasoning blocks or
// failed to swap the text would break here. It must not break the loop.
#[tokio::test]
async fn edit_plan_replaces_text_and_preserves_non_text_blocks() {
    use opencoder_core::{resolve_agent, ContentBlock, Message};
    use opencoder_llm::MockChatClient;

    let (evt_tx, _evt_rx) = mpsc::channel::<UiEvent>(8);
    let agent = resolve_agent("act").expect("act agent");
    let mut sess = SessionState::new(
        "edit-plan-test",
        agent,
        opencoder_core::Config::default(),
        std::sync::Arc::new(MockChatClient::new()) as std::sync::Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    );

    // Realistic plan-mode assistant shape: a Reasoning block followed by the
    // plan Text block.
    let mut msg = Message::assistant("a1");
    msg.blocks = vec![
        ContentBlock::Reasoning {
            text: "let me think".into(),
        },
        ContentBlock::Text {
            text: "original plan".into(),
        },
    ];
    sess.messages.push(msg);

    let should_break = process_cmd(
        UiCmd::EditPlan("edited plan text".to_string()),
        &mut sess,
        &evt_tx,
    )
    .await;
    assert!(!should_break, "EditPlan must not break the worker loop");

    // Exactly one assistant message, now carrying the edited plan.
    assert_eq!(sess.messages.len(), 1);
    let edited = &sess.messages[0];
    assert_eq!(edited.text(), "edited plan text", "text must be replaced");

    // The Reasoning block survives the edit.
    let has_reasoning = edited
        .blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Reasoning { text } if text == "let me think"));
    assert!(
        has_reasoning,
        "non-Text blocks must be preserved across the edit"
    );

    // Exactly one Text block remains (the original was dropped, not kept).
    let text_count = edited
        .blocks
        .iter()
        .filter(|b| matches!(b, ContentBlock::Text { .. }))
        .count();
    assert_eq!(
        text_count, 1,
        "old Text block must be replaced, not appended"
    );
}

#[test]
fn forward_event_throttles_delta_preserves_lifecycle() {
    // Channel with small capacity so we can fill it easily.
    let (tx, _rx) = mpsc::channel::<UiEvent>(2 * DELTA_MIN_CAPACITY + 1);

    // Fill the channel to near-capacity (leave <= DELTA_MIN_CAPACITY free).
    for _ in 0..DELTA_MIN_CAPACITY + 1 {
        tx.try_send(UiEvent::TurnDone("act".into())).unwrap();
    }
    // Now capacity() <= DELTA_MIN_CAPACITY — deltas should be dropped.
    assert!(tx.capacity() <= DELTA_MIN_CAPACITY);

    // TextDelta is droppable — forward_event should silently drop it.
    forward_event(&tx, SessionEvent::TextDelta("x".into()));
    // Capacity unchanged (event was dropped, not enqueued).
    assert!(tx.capacity() <= DELTA_MIN_CAPACITY);

    // SubagentChild wrapping TextDelta is also droppable.
    forward_event(
        &tx,
        SessionEvent::SubagentChild {
            id: "s1".into(),
            ev: Box::new(SessionEvent::TextDelta("y".into())),
        },
    );

    // SubagentStart is a lifecycle event — must always get through.
    forward_event(
        &tx,
        SessionEvent::SubagentStart {
            id: "s1".into(),
            kind: "explore".into(),
            prompt: "p".into(),
            child_session_id: "c1".into(),
        },
    );
    // The SubagentStart should have been enqueued (capacity decreased by 1).
    assert_eq!(tx.capacity(), DELTA_MIN_CAPACITY - 1);

    // SubagentEnd is a lifecycle event — must always get through.
    forward_event(
        &tx,
        SessionEvent::SubagentEnd {
            id: "s1".into(),
            ok: true,
            cancelled: false,
            summary: "done".into(),
        },
    );
    assert_eq!(tx.capacity(), DELTA_MIN_CAPACITY - 2);
}

#[test]
fn forward_event_never_drops_reasoning_delta() {
    // ReasoningDelta drives the `💭 Thinking` label and is NOT rebuilt from
    // the store on TurnDone (finalize_assistant only restores text), so it
    // must never be silently dropped even when the channel is saturated.
    let (tx, _rx) = mpsc::channel::<UiEvent>(2 * DELTA_MIN_CAPACITY + 1);

    // Fill the channel to near-capacity (leave <= DELTA_MIN_CAPACITY free).
    for _ in 0..DELTA_MIN_CAPACITY + 1 {
        tx.try_send(UiEvent::TurnDone("act".into())).unwrap();
    }
    assert!(tx.capacity() <= DELTA_MIN_CAPACITY);

    // ReasoningDelta is not droppable — must still be enqueued.
    forward_event(&tx, SessionEvent::ReasoningDelta("think".into()));
    // Capacity decreased by 1 because the event was enqueued, not dropped.
    assert_eq!(tx.capacity(), DELTA_MIN_CAPACITY - 1);

    // SubagentChild wrapping ReasoningDelta is also not droppable.
    forward_event(
        &tx,
        SessionEvent::SubagentChild {
            id: "s1".into(),
            ev: Box::new(SessionEvent::ReasoningDelta("child think".into())),
        },
    );
    assert_eq!(tx.capacity(), DELTA_MIN_CAPACITY - 2);
}

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

// EditAnnotation persists the /ann editor's submitted text as the session
// requirement. The text must land verbatim in `sess.requirement` and the
// worker loop must keep running (store is None in the test session, so the
// patch is skipped and the assertion is in-memory only).
#[tokio::test]
async fn edit_annotation_sets_requirement() {
    let (evt_tx, _evt_rx) = mpsc::channel::<UiEvent>(8);
    let mut sess = test_session("edit-annotation-set");

    let should_break = process_cmd(
        UiCmd::EditAnnotation("需要 tab:\tand CR\r\nraw".to_string()),
        &mut sess,
        &evt_tx,
    )
    .await;
    assert!(
        !should_break,
        "EditAnnotation must not break the worker loop"
    );
    assert_eq!(
        sess.requirement.as_deref(),
        Some("需要 tab:\tand CR\r\nraw"),
        "requirement must hold the text byte-for-byte"
    );
}

// A blank (whitespace-only) submit is an explicit clear: any previously
// stored requirement must be dropped in-memory.
#[tokio::test]
async fn edit_annotation_blank_clears_requirement() {
    let (evt_tx, _evt_rx) = mpsc::channel::<UiEvent>(8);
    let mut sess = test_session("edit-annotation-clear");
    sess.requirement = Some("old".into());

    let should_break = process_cmd(
        UiCmd::EditAnnotation("   \n\t ".to_string()),
        &mut sess,
        &evt_tx,
    )
    .await;
    assert!(
        !should_break,
        "EditAnnotation must not break the worker loop"
    );
    assert_eq!(
        sess.requirement, None,
        "blank submit must clear the requirement"
    );
}

#[tokio::test]
async fn ordered_forwarder_drops_only_repairable_parent_text() {
    let (tx, mut rx) = mpsc::channel::<UiEvent>(DELTA_MIN_CAPACITY + 1);
    tx.send(UiEvent::TurnDone("sentinel".into())).await.unwrap();
    assert_eq!(tx.capacity(), DELTA_MIN_CAPACITY);
    let (pending, forwarder) = spawn_ui_event_forwarder(tx);

    forward_event(&pending, SessionEvent::TextDelta("droppable".into()));
    forward_event(
        &pending,
        SessionEvent::SubagentChild {
            id: "s1".into(),
            ev: Box::new(SessionEvent::TextDelta("child text".into())),
        },
    );
    forward_event(&pending, SessionEvent::ReasoningDelta("thinking".into()));
    forward_event(&pending, SessionEvent::TranscriptReset(Vec::new()));
    drop(pending);
    forwarder.await.unwrap();

    assert!(matches!(rx.recv().await, Some(UiEvent::TurnDone(agent)) if agent == "sentinel"));
    assert!(matches!(
        rx.recv().await,
        Some(UiEvent::Session(SessionEvent::SubagentChild { ev, .. }))
            if matches!(*ev, SessionEvent::TextDelta(ref text) if text == "child text")
    ));
    assert!(matches!(
        rx.recv().await,
        Some(UiEvent::Session(SessionEvent::ReasoningDelta(text))) if text == "thinking"
    ));
    assert!(matches!(
        rx.recv().await,
        Some(UiEvent::Session(SessionEvent::TranscriptReset(messages))) if messages.is_empty()
    ));
    assert!(rx.try_recv().is_err(), "parent TextDelta must be shed");
}

#[test]
fn completed_assistant_text_is_scoped_to_messages_added_by_current_turn() {
    let mut sess = test_session("completed-text");
    sess.messages
        .push(opencoder_core::Message::assistant("old"));
    sess.messages.last_mut().unwrap().blocks = vec![opencoder_core::ContentBlock::Text {
        text: "old answer".into(),
    }];
    let floor = sess.messages.len();
    assert_eq!(completed_assistant_text(&sess, floor), None);

    sess.messages
        .push(opencoder_core::Message::user("u", "next"));
    sess.messages
        .push(opencoder_core::Message::assistant("new"));
    sess.messages.last_mut().unwrap().blocks = vec![opencoder_core::ContentBlock::Text {
        text: "complete new answer".into(),
    }];
    assert_eq!(
        completed_assistant_text(&sess, floor).as_deref(),
        Some("complete new answer")
    );
}

#[tokio::test]
async fn prompt_sends_reliable_completed_answer_before_turn_done() {
    use opencoder_llm::{LlmEvent, MockChatClient};

    let mock = MockChatClient::new().push_script(vec![
        LlmEvent::TextDelta("partial".into()),
        LlmEvent::Completed {
            text: "complete answer".into(),
            tool_calls: Vec::new(),
            usage: None,
        },
    ]);
    let mut sess = SessionState::new(
        "assistant-final",
        opencoder_core::resolve_agent("act").unwrap(),
        opencoder_core::Config::default(),
        Arc::new(mock),
        std::env::temp_dir(),
    );
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(32);

    assert!(
        !process_cmd(
            UiCmd::Prompt("question".into(), Vec::new()),
            &mut sess,
            &evt_tx
        )
        .await
    );
    let mut events = Vec::new();
    while let Ok(event) = evt_rx.try_recv() {
        events.push(event);
    }
    let final_idx = events
        .iter()
        .position(
            |event| matches!(event, UiEvent::AssistantFinal(text) if text == "complete answer"),
        )
        .expect("prompt must emit the reliable completed answer");
    let done_idx = events
        .iter()
        .position(|event| matches!(event, UiEvent::TurnDone(_)))
        .expect("prompt must emit TurnDone");
    assert!(
        final_idx < done_idx,
        "completion repair must precede TurnDone"
    );
}

#[tokio::test]
async fn ordered_forwarder_backpressures_without_losing_reliable_events() {
    let (tx, mut rx) = mpsc::channel::<UiEvent>(UI_EVENT_CAPACITY);
    for index in 0..UI_EVENT_CAPACITY {
        tx.try_send(UiEvent::TurnDone(format!("sentinel-{index}")))
            .unwrap();
    }
    let (pending, forwarder) = spawn_ui_event_forwarder(tx);
    forward_event(
        &pending,
        SessionEvent::ReasoningDelta("parent think".into()),
    );
    forward_event(
        &pending,
        SessionEvent::SubagentChild {
            id: "s1".into(),
            ev: Box::new(SessionEvent::ReasoningDelta("child think".into())),
        },
    );
    forward_event(
        &pending,
        SessionEvent::SubagentEnd {
            id: "s1".into(),
            ok: true,
            cancelled: false,
            summary: "done".into(),
        },
    );
    drop(pending);

    for index in 0..UI_EVENT_CAPACITY {
        assert!(matches!(
            rx.recv().await,
            Some(UiEvent::TurnDone(agent)) if agent == format!("sentinel-{index}")
        ));
    }
    assert!(matches!(
        rx.recv().await,
        Some(UiEvent::Session(SessionEvent::ReasoningDelta(text))) if text == "parent think"
    ));
    assert!(matches!(
        rx.recv().await,
        Some(UiEvent::Session(SessionEvent::SubagentChild { ev, .. }))
            if matches!(*ev, SessionEvent::ReasoningDelta(ref text) if text == "child think")
    ));
    assert!(matches!(
        rx.recv().await,
        Some(UiEvent::Session(SessionEvent::SubagentEnd { summary, .. })) if summary == "done"
    ));
    forwarder.await.unwrap();
}

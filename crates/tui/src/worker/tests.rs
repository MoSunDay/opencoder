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

// ── control-command worker semantics (pure prompt path) ─────────────────────

// The worker has no SwitchAgent/SwitchAndStart arms anymore: an agent switch
// or a clear-context fold arrives as a plain `UiCmd::Prompt` carrying the
// control-command text. `process_cmd` forwards it to `run_session`, whose
// idle short-circuit applies the command (no LLM call) and emits the
// lifecycle events the UI chip folds from.
#[tokio::test]
async fn prompt_control_cmd_switches_agent_without_llm_turn() {
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);
    let mut sess = test_session("switch-cmd");
    let _ = process_cmd(
        UiCmd::Prompt("/plan".into(), Vec::new()),
        &mut sess,
        &evt_tx,
    )
    .await;
    assert_eq!(sess.agent.name, "plan", "pure prompt switches the agent");
    let saw_switch = std::iter::from_fn(|| evt_rx.try_recv().ok())
        .any(|e| matches!(e, UiEvent::Session(SessionEvent::AgentSwitch(ref n)) if n == "plan"));
    assert!(saw_switch, "an AgentSwitch event must reach the UI bridge");
}

#[tokio::test]
async fn prompt_clear_context_folds_transcript_and_emits_reset() {
    use opencoder_core::{ContentBlock, Message};
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);
    let mut sess = test_session("clear-cmd");
    let mut say = Message::assistant("a1");
    say.blocks.push(ContentBlock::text("the latest say"));
    sess.messages.push(say);
    let _ = process_cmd(
        UiCmd::Prompt(crate::clear_confirm::CLEAR_CONTEXT_CMD.into(), Vec::new()),
        &mut sess,
        &evt_tx,
    )
    .await;
    assert_eq!(sess.messages.len(), 1, "transcript folds to one seed");
    let saw_reset = std::iter::from_fn(|| evt_rx.try_recv().ok())
        .any(|e| matches!(e, UiEvent::Session(SessionEvent::TranscriptReset(_))));
    assert!(
        saw_reset,
        "a TranscriptReset event must reach the UI bridge"
    );
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
// guards the plan editor edit path: an edit that dropped Reasoning blocks or
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

    // Realistic plan editor assistant shape: a Reasoning block followed by the
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
/// 让 forwarder 任务把 pending 队列处理到下一个 await 点（current_thread
/// 运行时下，几次 yield 足以让它消费完所有已入队事件）。
async fn pump_forwarder() {
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
}

/// 压力下被丢弃(shed)的 TextDelta 若携带 `'\n'`，下一个送达的 delta 必须
/// 带上分隔换行：被中断的回合没有 AssistantFinal 修复，行界一旦丢失就会
/// 永久冻结成「多行贴在一起」。
#[tokio::test]
async fn forwarder_shedding_preserves_line_breaks() {
    let (tx, mut rx) = mpsc::channel::<UiEvent>(DELTA_MIN_CAPACITY + 1);
    tx.send(UiEvent::TurnDone("sentinel".into())).await.unwrap();
    assert_eq!(tx.capacity(), DELTA_MIN_CAPACITY);
    let (pending, forwarder) = spawn_ui_event_forwarder(tx);

    forward_event(&pending, SessionEvent::TextDelta("head ".into()));
    forward_event(&pending, SessionEvent::TextDelta("sep\ntail2\n".into()));
    // 先让 forwarder 在低压下处理完（两个 delta 均被 shed）。
    pump_forwarder().await;
    // 再腾出容量：下一个 delta 不再处于丢弃阈值之下。
    assert!(matches!(rx.recv().await, Some(UiEvent::TurnDone(_))));
    forward_event(&pending, SessionEvent::TextDelta("tail3\n".into()));
    drop(pending);
    forwarder.await.unwrap();

    assert!(
        matches!(rx.recv().await, Some(UiEvent::Session(SessionEvent::TextDelta(t))) if t == "\ntail3\n"),
        "next delivered delta must carry the separator for the shed line break"
    );
    assert!(rx.try_recv().is_err());
}

/// shed 的块不含换行（同一行内的丢弃）→ 下一个 delta 原样送达，不得插入
/// 多余换行把一行拆成两行。
#[tokio::test]
async fn forwarder_shed_without_newline_adds_no_separator() {
    let (tx, mut rx) = mpsc::channel::<UiEvent>(DELTA_MIN_CAPACITY + 1);
    tx.send(UiEvent::TurnDone("sentinel".into())).await.unwrap();
    let (pending, forwarder) = spawn_ui_event_forwarder(tx);

    forward_event(&pending, SessionEvent::TextDelta("mid".into()));
    pump_forwarder().await;
    assert!(matches!(rx.recv().await, Some(UiEvent::TurnDone(_))));
    forward_event(&pending, SessionEvent::TextDelta("line\n".into()));
    drop(pending);
    forwarder.await.unwrap();

    assert!(
        matches!(rx.recv().await, Some(UiEvent::Session(SessionEvent::TextDelta(t))) if t == "line\n"),
        "no stray separator when the shed chunk carried no line break"
    );
}

/// 欠下的分隔换行不得越过回合边界泄漏：LlmRoundEnd 封口当前 Say 后，
/// 下一回合首个 delta 不得带前导空行。
#[tokio::test]
async fn forwarder_shed_separator_does_not_leak_past_round_end() {
    // +2 容量、2 个哨兵：弹出后余量越过阈值，交付阶段的 delta 不会被
    // 再次判定为可丢弃。
    let (tx, mut rx) = mpsc::channel::<UiEvent>(DELTA_MIN_CAPACITY + 2);
    for _ in 0..2 {
        tx.send(UiEvent::TurnDone("sentinel".into())).await.unwrap();
    }
    let (pending, forwarder) = spawn_ui_event_forwarder(tx);

    forward_event(&pending, SessionEvent::TextDelta("a\nb\n".into()));
    pump_forwarder().await;
    for _ in 0..2 {
        assert!(matches!(rx.recv().await, Some(UiEvent::TurnDone(_))));
    }
    forward_event(&pending, SessionEvent::LlmRoundEnd);
    forward_event(&pending, SessionEvent::TextDelta("next".into()));
    drop(pending);
    forwarder.await.unwrap();

    assert!(matches!(rx.recv().await, Some(UiEvent::Session(SessionEvent::LlmRoundEnd))));
    assert!(
        matches!(rx.recv().await, Some(UiEvent::Session(SessionEvent::TextDelta(t))) if t == "next"),
        "round boundary consumes the owed separator"
    );
}

/// 下一个 delta 自身以换行开头时不再叠加分隔符。
#[tokio::test]
async fn forwarder_shed_separator_not_doubled_when_next_starts_with_newline() {
    let (tx, mut rx) = mpsc::channel::<UiEvent>(DELTA_MIN_CAPACITY + 1);
    tx.send(UiEvent::TurnDone("sentinel".into())).await.unwrap();
    let (pending, forwarder) = spawn_ui_event_forwarder(tx);

    forward_event(&pending, SessionEvent::TextDelta("x\n".into()));
    pump_forwarder().await;
    assert!(matches!(rx.recv().await, Some(UiEvent::TurnDone(_))));
    forward_event(&pending, SessionEvent::TextDelta("\nrest".into()));
    drop(pending);
    forwarder.await.unwrap();

    assert!(
        matches!(rx.recv().await, Some(UiEvent::Session(SessionEvent::TextDelta(t))) if t == "\nrest"),
        "separator must not double when the next delta already starts with one"
    );
}

/// 中途压缩(TranscriptReset)把消息列表整体替换后，完成回合的可靠
/// AssistantFinal 仍须送达：修复地板要跟随重置后的消息数，而不是停留在
/// 压缩前的越界下标（旧代码 `get(floor..)` 返回 None → 修复静默丢失，
/// 被丢弃的 delta 行界就永久冻结）。
#[tokio::test]
async fn prompt_after_midrun_compaction_still_sends_completed_answer() {
    use opencoder_llm::{LlmEvent, MockChatClient, Usage};

    let mock = MockChatClient::new()
        .push_script(vec![
            LlmEvent::TextDelta("intermediate".into()),
            LlmEvent::Completed {
                text: "intermediate say".into(),
                tool_calls: vec![opencoder_llm::CompletedToolCall {
                    id: "c1".into(),
                    name: "bash".into(),
                    input: serde_json::json!({"command": "echo x"}),
                }],
                usage: Some(Usage {
                    input_tokens: 100_000,
                    output_tokens: 1,
                    total_tokens: 100_001,
                    ..Default::default()
                }),
            },
        ])
        // 压缩摘要调用（small_model 走同一个 client）。
        .push_script(vec![LlmEvent::Completed {
            text: "compaction summary".into(),
            tool_calls: Vec::new(),
            usage: None,
        }])
        // 压缩后的最终回答。
        .push_script(vec![LlmEvent::Completed {
            text: "final answer".into(),
            tool_calls: Vec::new(),
            usage: None,
        }]);
    let mut sess = SessionState::new(
        "compact-final",
        opencoder_core::resolve_agent("act").unwrap(),
        {
            let mut cfg = opencoder_core::Config::default();
            cfg.compaction.context_threshold = 50_000;
            cfg
        },
        Arc::new(mock),
        std::env::temp_dir(),
    );
    // 预置一段长转录，使 run 开始时的 message_floor 远大于压缩后的消息数。
    for i in 0..40 {
        sess.messages.push(opencoder_core::Message::user(
            "seed",
            &format!("seed message {i} {}", "x".repeat(80)),
        ));
    }
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(256);

    assert!(
        !process_cmd(
            UiCmd::Prompt("question".into(), Vec::new()),
            &mut sess,
            &evt_tx
        )
        .await
    );
    let mut saw_reset = false;
    let mut final_answer: Option<String> = None;
    while let Ok(event) = evt_rx.try_recv() {
        match event {
            UiEvent::Session(SessionEvent::TranscriptReset(_)) => saw_reset = true,
            UiEvent::AssistantFinal(text) => final_answer = Some(text),
            _ => {}
        }
    }
    assert!(saw_reset, "test premise: compaction actually ran mid-run");
    assert_eq!(
        final_answer.as_deref(),
        Some("final answer"),
        "reliable completed answer must survive the mid-run compaction"
    );
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

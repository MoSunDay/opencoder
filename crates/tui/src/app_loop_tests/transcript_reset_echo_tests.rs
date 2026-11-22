//! TranscriptReset turn-boundary tests: the in-flight prompt's echo must
// survive the view rebuild so the running turn keeps its user boundary
// (1 Turn = n Steps + Say; never merged into the pre-reset turn).

use super::*;

use crate::chat::ChatBlock;
use opencoder_core::Message;
use opencoder_session::SessionEvent;
use opencoder_store::{SessionMeta, TursoStore};

/// The user's regression: plan→act handoff (`/act_clear_context <tail>`
// submitted while idle). The submit path echoes the tail locally and starts
/// the run; the runner applies ClearContext, whose `TranscriptReset` rebuilds
/// the view from the folded transcript — recorded BEFORE the tail enters the
/// message list. Without echo restoration the new turn renders with no user
/// boundary (steps read as accumulated into the previous turn, Says glued).
#[tokio::test]
async fn transcript_reset_restores_in_flight_echo_below_rebuilt_view() {
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "sess-reset-echo".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut chat = ChatView::default();

    // --- Turn 1 (plan): user + one round with a tool + a Say, then done.
    crate::app_helpers::push_user(
        &mut chat,
        &mut Vec::new(),
        &mut None,
        "编写诊断工具",
        "编写诊断工具",
    );
    chat.begin_turn();
    chat.apply(&SessionEvent::LlmRoundStart {
        started_at_ms: 1_000,
    });
    chat.apply(&SessionEvent::ReasoningDelta("plan it".into()));
    chat.apply(&SessionEvent::ToolStart {
        id: "call-1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "ls"}),
    });
    chat.apply(&SessionEvent::ToolEnd {
        id: "call-1".into(),
        name: "bash".into(),
        output: "ok".into(),
        is_error: false,
        images: Vec::new(),
    });
    chat.apply(&SessionEvent::LlmRoundEnd);
    chat.apply(&SessionEvent::LlmRoundStart {
        started_at_ms: 2_000,
    });
    chat.apply(&SessionEvent::TextDelta("计划如下".into()));
    chat.apply(&SessionEvent::LlmRoundEnd);
    chat.apply(&SessionEvent::Done);

    // --- Submit `/act_clear_context 实现诊断工具` while idle: local echo +
    // begin_turn (mirrors app_submit / fire_clear_confirm).
    crate::app_helpers::push_user(
        &mut chat,
        &mut Vec::new(),
        &mut None,
        "实现诊断工具",
        "/act_clear_context 实现诊断工具",
    );
    chat.begin_turn();

    // --- Runner: TranscriptReset (folded transcript — the directive is
    // synthetic and renders nothing) then the act turn's rounds.
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut plan_skill_active = false;
    let mut admit = crate::queue_admitter::AdmitUiState::default();
    let mut running = true;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let mut question_menu = None;
    let question_hub = std::sync::Arc::new(opencoder_session::QuestionHub::default());
    let mut turn_gen = 0u64;
    let mut reset = crate::reset_rebuild::ResetRebuildState::default();

    // The reset now rebuilds OFF the loop: fold returns immediately (the
    // rebuild is spawned) and the rebuilt view installs via wait_done —
    // mirroring the app loop's reset-done arm.
    let flow = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::TranscriptReset(Vec::new()))),
        &mut chat,
        &store,
        "sess-reset-echo",
        &mut queue_items,
        &mut plan_skill_active,
        &mut admit,
        &mut running,
        &mut cancelled,
        &mut drain_pending,
        &mut skip_next_render,
        &mut follow,
        &cmd_tx,
        &mut cancel,
        &mut turn_gen,
        &mut evt_rx,
        &mut notepad,
        &mut question_menu,
        &question_hub,
        &mut reset,
    )
    .await;
    assert!(matches!(flow, LoopFlow::Proceed), "reset fold returns at once");
    assert!(reset.is_pending(), "the rebuild is in flight");
    if let Some(rebuilt) = reset.wait_done("sess-reset-echo").await {
        chat = rebuilt;
    }

    // The echo survived the rebuild as the LAST user boundary…
    let user_texts: Vec<usize> = chat
        .blocks
        .iter()
        .enumerate()
        .filter_map(|(i, b)| match b {
            ChatBlock::User { .. } => Some(i),
            _ => None,
        })
        .collect();
    assert_eq!(
        user_texts,
        vec![chat.blocks.len() - 2],
        "exactly one user block (the re-pushed echo) below the rebuilt view"
    );
    // …and the ladder floor sits below it.
    assert!(chat.turn_block_start >= chat.blocks.len() - 1);

    // --- The act turn streams: its steps and Say land in their own ladder.
    chat.apply(&SessionEvent::LlmRoundStart {
        started_at_ms: 3_000,
    });
    chat.apply(&SessionEvent::ReasoningDelta("execute".into()));
    chat.apply(&SessionEvent::ToolStart {
        id: "call-2".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "cat > diagnostics.html"}),
    });
    chat.apply(&SessionEvent::ToolEnd {
        id: "call-2".into(),
        name: "bash".into(),
        output: "written".into(),
        is_error: false,
        images: Vec::new(),
    });
    chat.apply(&SessionEvent::LlmRoundEnd);
    chat.apply(&SessionEvent::LlmRoundStart {
        started_at_ms: 4_000,
    });
    chat.apply(&SessionEvent::TextDelta("完成".into()));
    chat.apply(&SessionEvent::LlmRoundEnd);
    chat.apply(&SessionEvent::Done);

    let mut saw_group = false;
    let mut saw_say = false;
    for (i, b) in chat.blocks.iter().enumerate() {
        match b {
            ChatBlock::StepGroup { .. } => {
                assert!(!saw_say, "ladder must not come after the Say");
                saw_group = true;
                assert!(i > user_texts[0], "ladder sits below the echo boundary");
            }
            ChatBlock::Assistant { .. } => saw_say = true,
            ChatBlock::User { .. } => {
                assert!(i <= user_texts[0], "no stray user blocks after the echo");
            }
            _ => {}
        }
    }
    assert!(saw_group && saw_say, "turn renders N Steps + Say");
}

// Done clears the remembered echo: a LATER bare `/act_clear_context` (no
// tail) must not resurrect the previous turn's prompt as a user block.
#[tokio::test]
async fn done_clears_pending_echo_so_later_bare_reset_does_not_resurrect_it() {
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "sess-bare-reset".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut chat = ChatView::default();
    crate::app_helpers::push_user(&mut chat, &mut Vec::new(), &mut None, "first", "first");
    chat.begin_turn();
    chat.apply(&SessionEvent::LlmRoundStart { started_at_ms: 1 });
    chat.apply(&SessionEvent::TextDelta("ok".into()));
    chat.apply(&SessionEvent::LlmRoundEnd);
    chat.apply(&SessionEvent::Done);
    assert!(chat.pending_turn_echo.is_none(), "Done clears the echo");

    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut plan_skill_active = false;
    let mut admit = crate::queue_admitter::AdmitUiState::default();
    let mut running = true;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let mut question_menu = None;
    let question_hub = std::sync::Arc::new(opencoder_session::QuestionHub::default());
    let mut turn_gen = 0u64;
    let mut reset = crate::reset_rebuild::ResetRebuildState::default();
    let _ = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::TranscriptReset(Vec::new()))),
        &mut chat,
        &store,
        "sess-bare-reset",
        &mut queue_items,
        &mut plan_skill_active,
        &mut admit,
        &mut running,
        &mut cancelled,
        &mut drain_pending,
        &mut skip_next_render,
        &mut follow,
        &cmd_tx,
        &mut cancel,
        &mut turn_gen,
        &mut evt_rx,
        &mut notepad,
        &mut question_menu,
        &question_hub,
        &mut reset,
    )
    .await;
    if let Some(rebuilt) = reset.wait_done("sess-bare-reset").await {
        chat = rebuilt;
    }
    assert!(
        !chat
            .blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::User { .. })),
        "no user echo resurrected by a bare reset"
    );
}

// Remote (`opencode remote attach`) resets must NOT go through the off-loop
// rebuild actor: the remote client replays the whole view synchronously from
// the reset's own message list, and the remote/submitted flags survive.
#[tokio::test]
async fn remote_reset_still_replays_synchronously() {
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "sess-remote-reset".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut chat = ChatView {
        agent: "act".into(),
        ..Default::default()
    };
    chat.remote = true;
    chat.submitted = true;
    crate::app_helpers::push_user(
        &mut chat,
        &mut Vec::new(),
        &mut None,
        "u1",
        "stale-local-tail",
    );

    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut plan_skill_active = false;
    let mut admit = crate::queue_admitter::AdmitUiState::default();
    let mut running = false;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let mut question_menu = None;
    let question_hub = std::sync::Arc::new(opencoder_session::QuestionHub::default());
    let mut turn_gen = 0u64;
    let mut reset = crate::reset_rebuild::ResetRebuildState::default();

    let flow = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::TranscriptReset(vec![
            Message::user("r1", "remote replayed prompt"),
        ]))),
        &mut chat,
        &store,
        "sess-remote-reset",
        &mut queue_items,
        &mut plan_skill_active,
        &mut admit,
        &mut running,
        &mut cancelled,
        &mut drain_pending,
        &mut skip_next_render,
        &mut follow,
        &cmd_tx,
        &mut cancel,
        &mut turn_gen,
        &mut evt_rx,
        &mut notepad,
        &mut question_menu,
        &question_hub,
        &mut reset,
    )
    .await;

    assert!(matches!(flow, LoopFlow::Proceed));
    assert!(
        !reset.is_pending(),
        "remote resets never spawn an off-loop rebuild"
    );
    assert!(
        chat.remote && chat.submitted,
        "remote flags survive the inline replay"
    );
    let user_text = |b: &ChatBlock| match b {
        ChatBlock::User { rendered } => Some(
            rendered
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    };
    assert!(
        chat.blocks
            .iter()
            .any(|b| user_text(b).as_deref().is_some_and(|t| t.contains("remote replayed prompt"))),
        "the view is replaced synchronously from the reset's message list"
    );
    assert!(
        !chat
            .blocks
            .iter()
            .any(|b| user_text(b).as_deref().is_some_and(|t| t.contains("stale-local-tail"))),
        "the pre-reset local view is gone after the inline replay"
    );
}

/// Unit contract of `echo_clear_context_fire` (the confirm-path echo
/// discipline): a compound tail echoes and is remembered; a BARE command
/// records nothing and DROPS any echo residue left by an interrupted
/// in-flight turn (`cancel_running_turn` deliberately keeps pending rows).
#[test]
fn echo_clear_context_fire_variants() {
    // Compound: the tail echoes as the fresh turn's user boundary and is
    // remembered across the rebuild.
    let mut chat = ChatView::default();
    crate::app_helpers::echo_clear_context_fire(&mut chat, "/act_clear_context 实现诊断工具");
    // The User block is followed by the boundary marker push.
    assert!(
        chat.blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::User { .. })),
        "the compound tail echoes as a User block"
    );
    assert_eq!(chat.pending_turn_echo.as_deref(), Some("实现诊断工具"));

    // Bare: no echo is pushed and any remembered residue is cleared — the
    // fold discards the turn that produced it, so it must not resurface.
    let mut chat = ChatView {
        pending_turn_echo: Some("cancelled turn prompt".into()),
        ..Default::default()
    };
    crate::app_helpers::echo_clear_context_fire(&mut chat, "/act_clear_context");
    assert!(chat.pending_turn_echo.is_none());
    assert!(
        !chat
            .blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::User { .. })),
        "a bare fire never pushes a User block"
    );
}

/// The in-flight bare-clear regression: the previous prompt was submitted
/// (echo remembered) and the turn is still running when the armed guard is
/// manually confirmed. The fired BARE `/act_clear_context` must drop that
/// residue BEFORE the TranscriptReset lands — otherwise the rebuild pushes
/// the cancelled turn's prompt back as a resurrected User block.
#[tokio::test]
async fn bare_clear_fire_drops_in_flight_echo_before_rebuild() {
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "sess-bare-fire".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut chat = ChatView::default();

    // --- Turn 1 still in flight: the submitted prompt's echo is remembered.
    crate::app_helpers::push_user(
        &mut chat,
        &mut Vec::new(),
        &mut None,
        "旧任务提示词",
        "旧任务提示词",
    );
    chat.begin_turn();
    chat.apply(&SessionEvent::LlmRoundStart {
        started_at_ms: 1_000,
    });
    chat.apply(&SessionEvent::ReasoningDelta("mid-run".into()));
    assert_eq!(chat.pending_turn_echo.as_deref(), Some("旧任务提示词"));

    // --- Manual confirm: cancel_running_turn keeps the pending row, then
    // the BARE command fires and must fold the residue away.
    crate::app_helpers::echo_clear_context_fire(&mut chat, "/act_clear_context");
    assert!(
        chat.pending_turn_echo.is_none(),
        "bare fire drops the interrupted turn's echo residue"
    );

    // --- The fold's TranscriptReset rebuilds from the (now empty) view.
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let (_evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);
    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut plan_skill_active = false;
    let mut admit = crate::queue_admitter::AdmitUiState::default();
    let mut running = true;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = true;
    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let mut question_menu = None;
    let question_hub = std::sync::Arc::new(opencoder_session::QuestionHub::default());
    let mut turn_gen = 0u64;
    let mut reset = crate::reset_rebuild::ResetRebuildState::default();
    let _ = fold_ui_events(
        Some(UiEvent::Session(SessionEvent::TranscriptReset(Vec::new()))),
        &mut chat,
        &store,
        "sess-bare-fire",
        &mut queue_items,
        &mut plan_skill_active,
        &mut admit,
        &mut running,
        &mut cancelled,
        &mut drain_pending,
        &mut skip_next_render,
        &mut follow,
        &cmd_tx,
        &mut cancel,
        &mut turn_gen,
        &mut evt_rx,
        &mut notepad,
        &mut question_menu,
        &question_hub,
        &mut reset,
    )
    .await;
    if let Some(rebuilt) = reset.wait_done("sess-bare-fire").await {
        chat = rebuilt;
    }
    assert!(
        !chat
            .blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::User { .. })),
        "no User block may resurrect after a bare clear fold"
    );
    assert!(
        chat.pending_turn_echo.is_none(),
        "the rebuilt view carries no echo memory"
    );
}

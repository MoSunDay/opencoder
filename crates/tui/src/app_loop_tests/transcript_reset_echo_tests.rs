//! TranscriptReset turn-boundary tests: the in-flight prompt's echo must
// survive the view rebuild so the running turn keeps its user boundary
// (1 Turn = n Steps + Say; never merged into the pre-reset turn).

use super::*;

use crate::chat::ChatBlock;
use opencoder_session::SessionEvent;
use opencoder_store::{LibsqlStore, SessionMeta};

/// The user's regression: plan→act handoff (`/act_clear_context <tail>`
// submitted while idle). The submit path echoes the tail locally and starts
/// the run; the runner applies ClearContext, whose `TranscriptReset` rebuilds
/// the view from the folded transcript — recorded BEFORE the tail enters the
/// message list. Without echo restoration the new turn renders with no user
/// boundary (steps read as accumulated into the previous turn, Says glued).
#[tokio::test]
async fn transcript_reset_restores_in_flight_echo_below_rebuilt_view() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
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
        "写个贪吃蛇",
        "写个贪吃蛇",
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

    // --- Submit `/act_clear_context 实现贪吃蛇` while idle: local echo +
    // begin_turn (mirrors app_submit / fire_clear_confirm).
    crate::app_helpers::push_user(
        &mut chat,
        &mut Vec::new(),
        &mut None,
        "实现贪吃蛇",
        "/act_clear_context 实现贪吃蛇",
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

    let _ = fold_ui_events(
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
        &mut evt_rx,
        &mut notepad,
        &mut question_menu,
        &question_hub,
    )
    .await;

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
        input: serde_json::json!({"command": "cat > snake.html"}),
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
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
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
        &mut evt_rx,
        &mut notepad,
        &mut question_menu,
        &question_hub,
    )
    .await;
    assert!(
        !chat
            .blocks
            .iter()
            .any(|b| matches!(b, ChatBlock::User { .. })),
        "no user echo resurrected by a bare reset"
    );
}

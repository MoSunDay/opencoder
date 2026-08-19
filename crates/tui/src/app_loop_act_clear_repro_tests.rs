//! End-to-end reproduction of the reported plan→act regression (2026-08-19):
//! after a REAL plan turn produces a plan, both Shift+Tab and
//! `/act_clear_context` must hand the plan forward — never degrade to a plain
//! switch or wipe the context. Drives the real worker (`process_cmd`) and the
//! real UI glue (`fold_ui_events` consumption-time re-arm, `handle_switch_agent`,
//! `dispatch_slash_action`) so a regression in any layer fails loudly here.

use super::*;
use crate::chat::ChatView;
use crate::command::SlashAction;
use crate::worker::{process_cmd, UiCmd, UiEvent};
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_session::SessionState;
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use std::sync::Arc;
use tokio::sync::mpsc;

fn done_turn(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

/// Setup: an in-memory store with the session row + a plan-mode session whose
/// MockChatClient produces one plan answer.
async fn setup() -> (Arc<dyn Store>, SessionState, Arc<MockChatClient>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "repro".into(),
            agent: Some("plan".into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
    let mock =
        Arc::new(MockChatClient::new().push_script(vec![done_turn("## Plan\n1. do X\n2. do Y")]));
    let dir = tempfile::tempdir().unwrap();
    let sess = SessionState::new(
        "repro",
        opencoder_core::resolve_agent("plan").unwrap(),
        Config::default(),
        mock.clone(),
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();
    (store, sess, mock)
}

/// Drive the real plan turn (`UiCmd::Prompt`) and fold every emitted event
/// through `fold_ui_events` exactly like the TUI loop does.
async fn run_plan_turn(
    store: &Arc<dyn Store>,
    sess: &mut SessionState,
    chat: &mut ChatView,
    sid: &str,
) {
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(512);
    let quit = process_cmd(
        UiCmd::Prompt("implement feature X".into(), Vec::new()),
        sess,
        &evt_tx,
    )
    .await;
    assert!(!quit, "plan turn must not signal quit");
    // process_cmd awaited the forwarder: the whole batch is buffered in
    // evt_rx. Drop our sender so recv() terminates after the batch.
    drop(evt_tx);

    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut admit = crate::queue_admitter::AdmitUiState::default();
    let mut running = false;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = false;
    let (cmd_tx, _cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let question_hub = Arc::new(opencoder_session::QuestionHub::new());

    // Collect the full event batch, then fold it (loop-local mirrors of the
    // TUI's per-iteration state).
    while let Some(ev) = evt_rx.recv().await {
        let flow = fold_ui_events(
            Some(ev),
            chat,
            store,
            sid,
            &mut queue_items,
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
            &mut None,
            &question_hub,
        )
        .await;
        assert!(matches!(flow, LoopFlow::Proceed), "plan turn must not quit");
    }
}

/// After a real plan turn, Shift+Tab must fire a plan→act handoff
/// (`UiCmd::SwitchAndStart`) — not a plain `UiCmd::SwitchAgent` — and the
/// worker must collapse the transcript around the produced plan.
#[tokio::test]
async fn shift_tab_after_real_plan_turn_hands_plan_forward() {
    let (store, mut sess, _mock) = setup().await;
    let mut chat = ChatView {
        agent: "plan".into(),
        ..Default::default()
    };

    // Enter-submit the requirement in plan mode: the optimistic arm fires at
    // submit; the TurnDone(plan) consumption-time re-arm confirms it from the
    // persisted counter.
    run_plan_turn(&store, &mut sess, &mut chat, "repro").await;

    // The plan phase must have recorded a real requirement + snapshot.
    assert_eq!(sess.plan_input_count, 1, "plan requirement must be counted");
    assert!(
        sess.plan_snapshot
            .as_deref()
            .unwrap_or("")
            .contains("## Plan"),
        "plan snapshot must be captured by record()"
    );
    // The consumption-time re-arm must have armed the UI flag.
    assert!(
        chat.plan_submitted,
        "TurnDone(plan) must re-arm plan_submitted from the persisted counter"
    );

    // Shift+Tab plan→act while idle: handoff, not a plain switch.
    let mut running = false;
    let mut follow = false;
    let mut input = "".to_string();
    let mut cursor_idx = 0usize;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 0u64;
    let workdir = std::path::Path::new(".");
    let active_skill_body: Option<String> = None;

    let outcome = handle_switch_agent(
        "act".into(),
        false,
        &mut chat,
        &mut running,
        &mut follow,
        &mut input,
        &mut cursor_idx,
        &mut mode_flash,
        0,
        &cmd_tx,
        &mut cancel,
        &mut sys_tokens,
        workdir,
        &active_skill_body,
    )
    .await;
    assert!(matches!(outcome, SwitchOutcome::Proceed));

    let mut cmds = Vec::new();
    while let Ok(c) = cmd_rx.try_recv() {
        cmds.push(c);
    }
    // start_turn sends ResetCancel then the command.
    assert_eq!(cmds.len(), 2, "ResetCancel + SwitchAndStart expected");
    let start = cmds
        .into_iter()
        .find(|c| matches!(c, UiCmd::SwitchAndStart(..)))
        .expect("SwitchAndStart expected on Shift+Tab after a real plan");

    // The worker consumes SwitchAndStart: real handoff with the plan preserved.
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(512);
    let (name, extra) = match start {
        UiCmd::SwitchAndStart(n, e) => (n, e),
        _ => unreachable!(),
    };
    let quit = process_cmd(UiCmd::SwitchAndStart(name, extra), &mut sess, &evt_tx).await;
    assert!(!quit);
    let mut events: Vec<UiEvent> = Vec::new();
    while let Ok(ev) = evt_rx.try_recv() {
        events.push(ev);
    }
    assert!(
        events.iter().any(|e| matches!(
            e,
            UiEvent::Session(SessionEvent::TranscriptReset(ref msgs)) if msgs.len() == 1
        )),
        "Shift+Tab handoff must collapse the transcript to the single plan message"
    );
    let plan = sess.messages.first().map(|m| m.text()).unwrap_or_default();
    assert!(
        plan.contains("## Plan"),
        "the collapsed transcript must carry the produced plan, got: {plan}"
    );
}

/// After a real plan turn, `/act_clear_context` from plan mode must route
/// through the handoff (SwitchAndStart) — never the plain `/act_clear_context`
/// prompt that wipes the whole context.
#[tokio::test]
async fn act_clear_context_after_real_plan_turn_preserves_plan() {
    let (store, sess, _mock) = setup().await;
    let mut chat = ChatView {
        agent: "plan".into(),
        ..Default::default()
    };

    let mut queue_items: Vec<(i64, String)> = Vec::new();
    let mut admit = crate::queue_admitter::AdmitUiState::default();
    let mut running = false;
    let mut cancelled = false;
    let mut drain_pending = false;
    let mut skip_next_render = false;
    let mut follow = false;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut notepad: Option<crate::notepad::NotepadView> = None;
    let question_hub = Arc::new(opencoder_session::QuestionHub::new());

    // Run the plan turn through the real worker + fold_ui_events.
    {
        let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(512);
        let mut sess = sess;
        let quit = process_cmd(
            UiCmd::Prompt("implement feature X".into(), Vec::new()),
            &mut sess,
            &evt_tx,
        )
        .await;
        assert!(!quit);
        drop(evt_tx);
        while let Some(ev) = evt_rx.recv().await {
            let flow = fold_ui_events(
                Some(ev),
                &mut chat,
                &store,
                "repro",
                &mut queue_items,
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
                &mut None,
                &question_hub,
            )
            .await;
            assert!(matches!(flow, LoopFlow::Proceed));
        }
        // After the fold loop, session state must show a recorded requirement.
        assert_eq!(sess.plan_input_count, 1);
        assert!(chat.plan_submitted, "re-arm must fire from TurnDone(plan)");
    }

    // Dispatch /act_clear_context from plan mode.
    let mut input = String::new();
    let mut cursor_idx = 0usize;
    let mut mode_flash: Option<(String, u32)> = None;
    let mut sys_tokens = 0u64;
    let mut config = Config::default();
    let workdir = std::path::Path::new(".");
    let flow = dispatch_slash_action(
        SlashAction::ClearContext,
        &cmd_tx,
        &mut cancel,
        &mut chat,
        &mut running,
        &mut follow,
        &store,
        "repro",
        &mut None,
        &mut None,
        &mut None,
        &mut None,
        &mut None,
        &mut None,
        &mut None,
        &mut None,
        "plan",
        &mut input,
        &mut cursor_idx,
        &mut config,
        workdir,
        &mut mode_flash,
        0,
        &mut sys_tokens,
        &mut None,
        &mut None,
    )
    .await;
    assert!(matches!(flow, LoopFlow::Proceed));

    let mut cmds = Vec::new();
    while let Ok(c) = cmd_rx.try_recv() {
        cmds.push(c);
    }
    let start = cmds
        .into_iter()
        .find(|c| matches!(c, UiCmd::SwitchAndStart(..)))
        .expect("/act_clear_context must route through SwitchAndStart (handoff) after a real plan");
    match start {
        UiCmd::SwitchAndStart(n, _) => assert_eq!(n, "act", "handoff must target act mode"),
        _ => unreachable!(),
    }
}

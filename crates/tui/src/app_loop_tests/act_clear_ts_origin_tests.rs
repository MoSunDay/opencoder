//! ts-origin variant of the plan→act repro (2026-08-19): sessions started
//! from `ts` output keep the session row's agent column NULL by design, and
//! the TurnDone(plan) consumption-time re-arm used to require
//! `meta.agent == Some("plan")` — disarming Shift+Tab. After a REAL plan turn,
//! Shift+Tab must hand the plan forward (SwitchAndStart), never degrade to a
//! plain switch.

use super::*;
use crate::chat::ChatView;
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

/// Setup for the ts-origin variant: the session row's agent/model columns are
/// NULL (ts-origin rows keep them NULL by design), and the session is marked
/// `ts_origin` exactly like a session started from `ts` output.
async fn setup_ts() -> (Arc<dyn Store>, SessionState, Arc<MockChatClient>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "ts-repro".into(),
            agent: None,
            model: None,
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
        "ts-repro",
        opencoder_core::resolve_agent("plan").unwrap(),
        Config::default(),
        mock.clone(),
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created()
    .ts_origin();
    (store, sess, mock)
}

/// Drive the real plan turn (`UiCmd::Prompt`) and fold every emitted event
/// through `fold_ui_events` exactly like the TUI loop does.
async fn run_plan_turn(store: &Arc<dyn Store>, sess: &mut SessionState, chat: &mut ChatView) {
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

    while let Some(ev) = evt_rx.recv().await {
        let flow = fold_ui_events(
            Some(ev),
            chat,
            store,
            "ts-repro",
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

/// The reported user scenario: a ts-origin session (row agent=NULL) switches
/// to plan, submits a requirement, and after the plan turn completes Shift+Tab
/// must hand the plan forward — not degrade to a plain switch. The
/// TurnDone(plan) re-arm must arm from the persisted counter/snapshot even
/// though the row's agent column is NULL.
#[tokio::test]
async fn shift_tab_ts_origin_session_hands_plan_forward() {
    let (store, mut sess, _mock) = setup_ts().await;
    let mut chat = ChatView {
        agent: "plan".into(),
        ..Default::default()
    };

    run_plan_turn(&store, &mut sess, &mut chat).await;

    // The plan phase recorded a real requirement, and the consumption-time
    // re-arm armed the UI flag despite the NULL agent column.
    assert_eq!(sess.plan_input_count, 1, "plan requirement must be counted");
    assert!(
        chat.plan_submitted,
        "TurnDone(plan) must re-arm plan_submitted even with a NULL agent column"
    );

    // Shift+Tab plan→act while idle: handoff (SwitchAndStart), not a plain
    // SwitchAgent.
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
        &mut None, // last_switch_sent dedup baseline
    )
    .await;
    assert!(matches!(outcome, SwitchOutcome::Proceed));

    let mut cmds = Vec::new();
    while let Ok(c) = cmd_rx.try_recv() {
        cmds.push(c);
    }
    // start_turn sends ResetCancel then the command.
    assert_eq!(cmds.len(), 2, "ResetCancel + SwitchAndStart expected");
    assert!(
        cmds.into_iter()
            .any(|c| matches!(c, UiCmd::SwitchAndStart(..))),
        "ts-origin Shift+Tab after a real plan must emit SwitchAndStart (handoff)"
    );
}

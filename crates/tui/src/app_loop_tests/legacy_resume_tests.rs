//! Regression: the deployed-binary legacy-session gap (2026-08-19). A session
//! created BEFORE the plan-phase columns existed (deployed `860831d`) persists
//! `plan_input_count=0` / `plan_snapshot=NULL` even when the plan agent
//! produced a real plan. On resume both must be recovered from the transcript
//! (phase-bounded: only a plan-agent assistant answer counts), so a restarted
//! TUI re-arms the plan→act handoff and both Shift+Tab and `/act_clear_context`
//! hand the plan forward instead of degrading to a plain switch or wiping the
//! whole context. Drives the real `resume`, the real `initial_chat_view` re-arm
//! and the real `handle_switch_agent` / `dispatch_slash_action` glue.

use super::*;
use crate::command::SlashAction;
use crate::worker::UiCmd;
use opencoder_core::{ContentBlock, Message};
use opencoder_llm::MockChatClient;
use opencoder_session::resume;
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use std::sync::Arc;
use tokio::sync::mpsc;

fn assistant(id: &str, agent: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m.agent = Some(agent.into());
    m
}

/// Legacy store: session meta in plan mode with EMPTY plan-phase columns, plus
/// a transcript that ends in a real plan-agent answer.
async fn setup_legacy(id: &str) -> (Arc<dyn Store>, opencoder_session::SessionState) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: id.into(),
            agent: Some("plan".into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            plan_snapshot: None,
            plan_input_count: 0,
            ..Default::default()
        })
        .await
        .unwrap();
    // Act-phase exchange first, then the plan phase that produced the plan —
    // the deployed-binary shape.
    store
        .append_messages(
            id,
            &[
                Message::user("u1", "do task X"),
                assistant("a1", "act", "task done"),
                Message::user("u2", "plan feature Y"),
                assistant("a2", "plan", "## Plan\n1. do X\n2. do Y"),
            ],
        )
        .await
        .unwrap();
    let sess = resume(
        store.clone(),
        id,
        Config::default(),
        Arc::new(MockChatClient::new()),
        tempfile::tempdir().unwrap().path().to_path_buf(),
    )
    .await
    .unwrap();
    (store, sess)
}

/// The resumed legacy session must re-arm `plan_submitted` from the recovered
/// snapshot, then Shift+Tab must fire `SwitchAndStart` (handoff) — never a
/// plain `SwitchAgent`.
#[tokio::test]
async fn legacy_resume_shift_tab_hands_plan_forward() {
    let (store, sess) = setup_legacy("legacy-tab").await;
    assert_eq!(
        sess.plan_input_count, 1,
        "resume must backfill the legacy plan counter"
    );
    assert!(
        sess.plan_snapshot
            .as_deref()
            .unwrap_or("")
            .contains("## Plan"),
        "resume must recover the legacy plan snapshot"
    );

    // The real startup re-arm path (app_helpers::initial_chat_view).
    let mut chat = crate::app_helpers::initial_chat_view(&sess, &store).await;
    assert!(
        chat.plan_submitted,
        "initial_chat_view must re-arm plan_submitted from the recovered snapshot"
    );

    let mut running = false;
    let mut follow = false;
    let mut input = "".to_string();
    let mut cursor_idx = 0usize;
    let mut mode_flash: Option<(String, u32)> = None;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
    let mut sys_tokens = 0u64;
    let workdir = std::path::Path::new(".");

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
        &None,
        &mut None,
    )
    .await;
    assert!(matches!(outcome, SwitchOutcome::Proceed));

    let mut cmds = Vec::new();
    while let Ok(c) = cmd_rx.try_recv() {
        cmds.push(c);
    }
    let start = cmds
        .into_iter()
        .find(|c| matches!(c, UiCmd::SwitchAndStart(..)))
        .expect("Shift+Tab on a resumed legacy plan session must hand off, not plain-switch");
    match start {
        UiCmd::SwitchAndStart(n, _) => assert_eq!(n, "act", "handoff must target act mode"),
        _ => unreachable!(),
    }
}

/// The resumed legacy session re-arms, then `/act_clear_context` from plan
/// mode must route through `SwitchAndStart` (handoff) — never the blank
/// `/act_clear_context` prompt that wipes the whole context.
#[tokio::test]
async fn legacy_resume_act_clear_context_preserves_plan() {
    let (store, sess) = setup_legacy("legacy-clear").await;
    let mut chat = crate::app_helpers::initial_chat_view(&sess, &store).await;
    assert!(
        chat.plan_submitted,
        "re-arm must fire from the recovered snapshot"
    );

    let mut running = false;
    let mut follow = false;
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let mut cancel = CancellationToken::new();
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
        "legacy-clear",
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
        .expect("/act_clear_context on a resumed legacy plan session must hand off, not clear");
    match start {
        UiCmd::SwitchAndStart(n, _) => assert_eq!(n, "act", "handoff must target act mode"),
        _ => unreachable!(),
    }
}

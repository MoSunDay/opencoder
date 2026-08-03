//! Regression tests for the `/task`-visible mode persistence fix.
//!
//! The TUI key handler (Alt+Tab / Ctrl+T) switches agent mode via
//! `UiCmd::SwitchAgent` / `UiCmd::SwitchAndStart`. Previously those paths only
//! mutated the in-memory `SessionState` and appended an `AgentSwitch` event —
//! which nothing replays — so `sessions.agent` stayed at the first-recorded
//! mode: an act-mode task exited via Ctrl+T still resumed showing `[plan]`,
//! and the `/task` picker read the stale mode.
//!
//! These tests prove the fix end-to-end:
//!  1. `SwitchAgent` persists the new agent to the store **and** a resumed
//!     session honors it instead of reverting to the stale stored mode.
//!  2. The plan→act `SwitchAndStart` handoff persists `act` so a resumed task
//!     shows `[act]` after execution.
use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message};
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_session::{resume, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use opencoder_tui::worker::{process_cmd, UiCmd, UiEvent};
use tokio::sync::mpsc;

/// Serialize tests that scrub process-wide proxy env vars (mirrors the
/// convention in `worker::tests`).
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn clear_proxy_env() {
    for v in &["OPENCODER_PROXY", "ALL_PROXY", "HTTPS_PROXY", "HTTP_PROXY"] {
        std::env::remove_var(v);
    }
}

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn assistant_with_text(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m
}

fn text_done(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn switch_agent_persists_mode_and_survives_resume() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_proxy_env();

    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "mode-switch".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let (tx, _rx) = mpsc::channel::<UiEvent>(8);
    let mut sess = SessionState::new(
        "mode-switch",
        resolve_agent("act").expect("act agent"),
        Config::default(),
        Arc::new(MockChatClient::new()) as Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    )
    .with_store(store.clone());
    assert_eq!(sess.agent.name, "act", "precondition");

    let quit = process_cmd(UiCmd::SwitchAgent("plan".into()), &mut sess, &tx).await;
    assert!(!quit, "SwitchAgent must not break the worker loop");
    assert_eq!(sess.agent.name, "plan", "in-memory mode swapped");

    // (a) persisted to the store so resume()/the /task picker read it.
    let meta = store
        .get_session("mode-switch")
        .await
        .unwrap()
        .expect("session row exists");
    assert_eq!(
        meta.agent.as_deref(),
        Some("plan"),
        "store must record the switched mode"
    );

    // (b) a resumed session honors the switch instead of reverting to act.
    let resumed = resume(
        store.clone(),
        "mode-switch",
        Config::default(),
        Arc::new(MockChatClient::new()) as Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    )
    .await
    .expect("resume succeeds");
    assert_eq!(
        resumed.agent.name, "plan",
        "resume must honor the persisted mode switch, not revert to act"
    );
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn switch_and_start_handoff_persists_act_mode() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_proxy_env();

    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "handoff-mode".into(),
            agent: Some("plan".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    // The act turn returns one completed text turn with no tool calls, so the
    // run loop settles after a single LLM call.
    let mock =
        Arc::new(MockChatClient::new().push_script(vec![text_done("starting implementation")]));
    let (tx, mut rx) = mpsc::channel::<UiEvent>(64);
    let mut sess = SessionState::new(
        "handoff-mode",
        resolve_agent("plan").expect("plan agent"),
        Config::default(),
        mock.clone(),
        std::env::temp_dir(),
    )
    .with_store(store.clone());
    sess.messages = vec![
        Message::user("u1", "implement feature X"),
        assistant_with_text("a1", "## Plan\n1. do X\n2. do Y"),
    ];

    let quit = process_cmd(
        UiCmd::SwitchAndStart("act".into(), "".into()),
        &mut sess,
        &tx,
    )
    .await;
    assert!(!quit, "SwitchAndStart must not break the worker loop");
    assert_eq!(sess.agent.name, "act", "in-memory mode swapped");
    let _ = rx.recv().await; // AgentSwitch event forwarded by the worker

    let meta = store
        .get_session("handoff-mode")
        .await
        .unwrap()
        .expect("session row exists");
    assert_eq!(
        meta.agent.as_deref(),
        Some("act"),
        "handoff must persist act mode"
    );

    let resumed = resume(
        store.clone(),
        "handoff-mode",
        Config::default(),
        Arc::new(MockChatClient::new()) as Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    )
    .await
    .expect("resume succeeds");
    assert_eq!(
        resumed.agent.name, "act",
        "a resumed post-execution task must show [act], not the stale [plan]"
    );
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn switch_agent_to_plan_resets_plan_input_count() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_proxy_env();

    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "plan-reset".into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();

    let (tx, _rx) = mpsc::channel::<UiEvent>(8);
    let mut sess = SessionState::new(
        "plan-reset",
        resolve_agent("act").expect("act agent"),
        Config::default(),
        Arc::new(MockChatClient::new()) as Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    )
    .with_store(store.clone());

    // Pre-seed a non-zero plan-input count, as if prior plan-mode turns
    // occurred. Switching back to plan via the TUI path must reset it, so the
    // "submit your plan" reminder logic starts from a fresh phase — matching
    // the `/plan` slash-command path (control_cmd::apply).
    sess.plan_input_count = 5;

    let quit = process_cmd(UiCmd::SwitchAgent("plan".into()), &mut sess, &tx).await;
    assert!(!quit, "SwitchAgent must not break the worker loop");
    assert_eq!(sess.agent.name, "plan", "in-memory mode swapped");
    assert_eq!(
        sess.plan_input_count, 0,
        "switching to plan must reset the plan-input counter, mirroring control_cmd::apply"
    );
}

//! Regression tests for the `/model` switch persistence fix.
//!
//! The TUI `/model` menu hot-swaps the model at a turn boundary via
//! `UiCmd::ReloadConfig`. Previously the swap touched only the in-memory
//! `SessionState`; the persisted `sessions.model` column was never updated,
//! so the next `resume()` (`opencode -s <id>`) reverted the model to the
//! stale stored value.
//!
//! These tests prove the fix end-to-end:
//!  1. `ReloadConfig` persists the new model to the store **and** forwards a
//!     `SessionEvent::ModelSwitch` event (the worker's direct responsibility).
//!  2. A resumed session honors the persisted switch instead of reverting.
use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ProviderConfig};
use opencoder_llm::MockChatClient;
use opencoder_session::{resume, SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use opencoder_tui::worker::{process_cmd, UiCmd, UiEvent};
use tokio::sync::mpsc;

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn switched_config() -> Config {
    Config {
        model: "openai/test-model".into(),
        provider: ProviderConfig {
            api_key: Some("k".into()),
            ..Default::default()
        },
        ..Config::default()
    }
}

#[tokio::test]
async fn reload_config_persists_model_and_emits_model_switch_event() {

    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "persist".into(),
            agent: Some("act".into()),
            model: Some(Config::default().model.clone()),
            ..Default::default()
        })
        .await
        .unwrap();

    let (tx, mut rx) = mpsc::channel::<UiEvent>(8);
    let mut sess = SessionState::new(
        "persist",
        resolve_agent("act").expect("act agent"),
        Config::default(),
        Arc::new(MockChatClient::new()) as Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    )
    .with_store(store.clone());

    assert_eq!(sess.model, "gpt-4o-mini", "precondition: default model");

    let broke = process_cmd(
        UiCmd::ReloadConfig(Box::new(switched_config())),
        &mut sess,
        &tx,
    )
    .await;
    assert!(!broke, "ReloadConfig must not break the worker loop");
    assert_eq!(sess.model, "test-model", "in-memory model swapped");

    // (a) persisted to the store so resume() honors it.
    let meta = store
        .get_session("persist")
        .await
        .unwrap()
        .expect("session row exists");
    assert_eq!(
        meta.model.as_deref(),
        Some("openai/test-model"),
        "store must record the switched model (full provider/id form)"
    );

    // (b) a ModelSwitch event is forwarded to the UI.
    let ev = rx.recv().await.expect("a ModelSwitch event was forwarded");
    match ev {
        UiEvent::Session(SessionEvent::ModelSwitch(m)) => {
            // The display marker carries the bare model id (no provider
            // prefix) so it matches the status bar (issue #1). The store
            // column above keeps the full provider/id form for resume.
            assert_eq!(m, "test-model", "ModelSwitch carries the bare model id");
        }
        _ => panic!("expected ModelSwitch event, got a different UiEvent variant"),
    }
}

#[tokio::test]
async fn model_switch_survives_resume() {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: "switch-resume".into(),
            agent: Some("act".into()),
            model: Some(Config::default().model.clone()),
            ..Default::default()
        })
        .await
        .unwrap();

    let (tx, mut rx) = mpsc::channel::<UiEvent>(8);
    let mut sess = SessionState::new(
        "switch-resume",
        resolve_agent("act").expect("act agent"),
        Config::default(),
        Arc::new(MockChatClient::new()) as Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    )
    .with_store(store.clone());

    process_cmd(
        UiCmd::ReloadConfig(Box::new(switched_config())),
        &mut sess,
        &tx,
    )
    .await;
    // drain the ModelSwitch event forwarded by the worker
    let _ = rx.recv().await.expect("ModelSwitch event forwarded");

    // Resume the *same* session with the OLD default config (as
    // `opencode -s <id>` would pass). resume() must prefer the persisted
    // model rather than reverting to gpt-4o-mini.
    let resumed = resume(
        store.clone(),
        "switch-resume",
        Config::default(),
        Arc::new(MockChatClient::new()) as Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    )
    .await
    .expect("resume succeeds");

    assert_eq!(
        resumed.model, "test-model",
        "resume must honor the persisted model switch, not revert to gpt-4o-mini"
    );
}

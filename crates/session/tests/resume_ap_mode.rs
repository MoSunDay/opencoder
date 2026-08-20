//! Session-scoped autopilot-mode override on resume.
//!
//! `/ap` "session-only" persists its choice to `sessions.autopilot_mode`.
//! On resume the stored value must be restored into
//! `SessionState::ap_mode_override` and win over the global
//! `config.autopilot.mode` at the runner's post-task dispatch point
//! (`effective_ap_mode`). NULL follows the global config; unknown values
//! warn and are ignored (no override, config mode governs).

use std::sync::Arc;

use opencoder_core::{ApMode, AutoPilotConfig, Config, Message};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_session::resume;
use opencoder_store::{LibsqlStore, SessionMeta, Store};

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config_with_mode(mode: ApMode) -> Config {
    Config {
        model: "m/g".into(),
        autopilot: AutoPilotConfig {
            mode,
            ..AutoPilotConfig::default()
        },
        ..Config::default()
    }
}

fn mock_client() -> Arc<dyn ChatStream> {
    Arc::new(MockChatClient::new())
}

/// Seed a session row with the given `autopilot_mode` column value plus one
/// message (resume fidelity: a real row always carries history).
async fn seed(store: &Arc<dyn Store>, id: &str, autopilot_mode: Option<&str>) {
    store
        .create_session(&SessionMeta {
            id: id.into(),
            title: Some("ap-mode".into()),
            agent: Some("act".into()),
            model: Some("m/g".into()),
            autopilot_mode: autopilot_mode.map(String::from),
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
            plan_snapshot: None,
            plan_input_count: 0,
        })
        .await
        .unwrap();
    store
        .append_message(id, &Message::user("u1", "hello"))
        .await
        .unwrap();
}

async fn resume_with(
    store: Arc<dyn Store>,
    id: &str,
    config: Config,
) -> opencoder_session::SessionState {
    let dir = tempfile::tempdir().unwrap();
    resume(store, id, config, mock_client(), dir.path().to_path_buf())
        .await
        .unwrap()
}

/// A stored `ap` override beats a global config of `off`.
#[tokio::test]
async fn resume_restores_ap_mode_override() {
    let store = mem_store().await;
    seed(&store, "s-ap", Some("ap")).await;

    let state = resume_with(store, "s-ap", config_with_mode(ApMode::Off)).await;
    assert_eq!(
        state.ap_mode_override,
        Some(ApMode::Ap),
        "stored sessions.autopilot_mode=ap must restore the override"
    );
    assert_eq!(
        state.effective_ap_mode(),
        ApMode::Ap,
        "the session override wins over config.autopilot.mode=off"
    );
}

/// NULL column = no override: the global config governs.
#[tokio::test]
async fn resume_null_ap_mode_follows_global_config() {
    let store = mem_store().await;
    seed(&store, "s-null", None).await;

    let state = resume_with(store, "s-null", config_with_mode(ApMode::Review)).await;
    assert_eq!(
        state.ap_mode_override, None,
        "NULL autopilot_mode must not create an override"
    );
    assert_eq!(
        state.effective_ap_mode(),
        ApMode::Review,
        "NULL column follows config.autopilot.mode"
    );
}

/// Unknown column values are ignored (warn + fall back to the config mode).
#[tokio::test]
async fn resume_ignores_unknown_ap_mode_value() {
    let store = mem_store().await;
    seed(&store, "s-bogus", Some("bogus")).await;

    let state = resume_with(store, "s-bogus", config_with_mode(ApMode::Ap)).await;
    assert_eq!(
        state.ap_mode_override, None,
        "an unknown mode string must not produce an override"
    );
    assert_eq!(
        state.effective_ap_mode(),
        ApMode::Ap,
        "unknown value falls back to the config mode"
    );
}

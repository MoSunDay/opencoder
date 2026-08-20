//! `UiCmd::ApModeSwitch` coverage: the `/ap` confirm dialog's `y` and `n`
//! both land here — the session override must be pinned, the in-memory
//! config mirrored, and `sessions.autopilot_mode` persisted so resume
//! honors it.

use super::*;

use opencoder_core::ApMode;
use opencoder_llm::MockChatClient;
use opencoder_store::{LibsqlStore, SessionMeta, Store};

/// Minimal session (mirrors `tests_reload.rs`'s harness) with the given
/// global-config autopilot mode.
fn make_sess(id: &str, mode: ApMode) -> SessionState {
    let agent = resolve_agent("act").expect("act agent");
    let mut config = Config::default();
    config.autopilot.mode = mode;
    SessionState::new(
        id,
        agent,
        config,
        std::sync::Arc::new(MockChatClient::new()) as std::sync::Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    )
}

/// `/ap` on → the override is pinned, the in-memory config mirrored, and the
/// `sessions.autopilot_mode` column persisted for resume.
#[tokio::test]
async fn ap_mode_switch_pins_override_and_persists_column() {
    let (evt_tx, _evt_rx) = mpsc::channel::<UiEvent>(8);
    let store: std::sync::Arc<dyn Store> =
        std::sync::Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut sess = make_sess("ap-switch-on", ApMode::Off).with_store(store.clone());
    store
        .create_session(&SessionMeta {
            id: sess.id.clone(),
            ..Default::default()
        })
        .await
        .unwrap();

    let quit = process_cmd(UiCmd::ApModeSwitch(ApMode::Ap), &mut sess, &evt_tx).await;
    assert!(!quit, "ApModeSwitch must not break the worker loop");
    assert_eq!(sess.ap_mode_override, Some(ApMode::Ap), "override pinned");
    assert_eq!(
        sess.config.autopilot.mode,
        ApMode::Ap,
        "in-memory config mirrored"
    );
    assert_eq!(
        store
            .get_session(&sess.id)
            .await
            .unwrap()
            .and_then(|m| m.autopilot_mode),
        Some("ap".to_string()),
        "sessions.autopilot_mode persisted for resume"
    );
    assert_eq!(sess.effective_ap_mode(), ApMode::Ap);
}

/// Switching to Off from a globally-Ap config (existing "ap" column): the
/// session override wins over the global config at dispatch time.
#[tokio::test]
async fn ap_mode_switch_off_overrides_global_ap_config() {
    let (evt_tx, _evt_rx) = mpsc::channel::<UiEvent>(8);
    let store: std::sync::Arc<dyn Store> =
        std::sync::Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut sess = make_sess("ap-switch-off", ApMode::Ap).with_store(store.clone());
    store
        .create_session(&SessionMeta {
            id: sess.id.clone(),
            autopilot_mode: Some("ap".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

    let quit = process_cmd(UiCmd::ApModeSwitch(ApMode::Off), &mut sess, &evt_tx).await;
    assert!(!quit);
    assert_eq!(sess.ap_mode_override, Some(ApMode::Off));
    assert_eq!(sess.config.autopilot.mode, ApMode::Off);
    let row = store.get_session(&sess.id).await.unwrap().expect("row");
    assert_eq!(row.autopilot_mode.as_deref(), Some("off"));
    assert_eq!(
        sess.effective_ap_mode(),
        ApMode::Off,
        "override beats the global ap config"
    );
}

/// No store attached (store `None`): the persist patch is silently skipped —
/// no panic, the in-memory override still applies, no event fires.
#[tokio::test]
async fn ap_mode_switch_without_store_skips_persist_silently() {
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(8);
    let mut sess = make_sess("ap-no-store", ApMode::Review);
    assert!(sess.store.is_none(), "harness: no store attached");

    let quit = process_cmd(UiCmd::ApModeSwitch(ApMode::Off), &mut sess, &evt_tx).await;
    assert!(!quit);
    assert_eq!(sess.ap_mode_override, Some(ApMode::Off));
    assert_eq!(sess.effective_ap_mode(), ApMode::Off);
    assert!(
        evt_rx.try_recv().is_err(),
        "a pure mode switch emits no event"
    );
}

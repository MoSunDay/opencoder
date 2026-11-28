use std::sync::Arc;

use super::*;
use opencoder_core::{resolve_agent, Config, ConfigMode, ModeConfig, ModesConfig, ProviderConfig};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_session::SessionEvent;
use opencoder_store::{SessionModelOverrides, Store, TursoStore};

fn config(shared: &str, act: &str) -> Config {
    Config {
        model: shared.into(),
        provider: ProviderConfig {
            base_url: "https://example.invalid/v1".into(),
            api_key: Some("test-placeholder".into()),
            ..Default::default()
        },
        modes: ModesConfig {
            act: Some(ModeConfig {
                model: Some(act.into()),
                context_limit: Some(15_000),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn session(id: &str, config: Config, store: Arc<TursoStore>) -> SessionState {
    SessionState::new(
        id,
        resolve_agent("act").unwrap(),
        config,
        Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
        std::env::temp_dir(),
    )
    .with_store(store)
}

fn apply_command(
    config: Config,
    selection: Option<(ConfigMode, String)>,
    provider_changed: bool,
) -> (UiCmd, mpsc::UnboundedReceiver<Result<(), String>>) {
    let (ack, completion) = mpsc::unbounded_channel();
    (
        UiCmd::ApplySession {
            config: Box::new(config),
            selection,
            provider_changed,
            ack,
        },
        completion,
    )
}

fn env_command(config: Config) -> (UiCmd, mpsc::UnboundedReceiver<Result<(), String>>) {
    let (ack, completion) = mpsc::unbounded_channel();
    (
        UiCmd::ReloadEnv {
            config: Box::new(config),
            ack,
        },
        completion,
    )
}

#[tokio::test]
async fn set_session_model_persists_act_override_without_config_file_writes() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let workdir = tempfile::tempdir().unwrap();
    let next = config("openai/shared", "openai/act");
    let mut sess = session("model-command", next.clone(), store.clone());
    sess.working_dir = workdir.path().to_path_buf();
    let original_client = Arc::as_ptr(&sess.client);
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(8);
    let (command, mut completion) = apply_command(
        next,
        Some((ConfigMode::Act, "deepseek/session-act".into())),
        false,
    );

    let quit = process_cmd(command, &mut sess, &evt_tx, 0).await;

    assert!(!quit);
    assert_eq!(completion.recv().await, Some(Ok(())));
    assert_eq!(sess.config.model, "openai/shared");
    assert_eq!(sess.effective_model(), "deepseek/session-act");
    assert_eq!(Arc::as_ptr(&sess.client), original_client);
    assert_eq!(
        store
            .get_session_model_overrides("model-command")
            .await
            .unwrap()
            .unwrap()
            .act
            .as_deref(),
        Some("deepseek/session-act")
    );
    assert!(!workdir.path().join("opencoder.json").exists());
    assert!(matches!(
        evt_rx.recv().await,
        Some(UiEvent::Session(SessionEvent::ModelSwitch(model))) if model == "session-act"
    ));
}

#[tokio::test]
async fn shared_model_command_persists_override_without_touching_base_or_client() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let mut next = config("openai/shared", "openai/act");
    next.modes.shared = ModeConfig {
        model: Some("openai/config-shared".into()),
        ..Default::default()
    };
    let mut sess = session("shared-command", next.clone(), store.clone());
    sess.agent = resolve_agent("build").unwrap();
    let original_client = Arc::as_ptr(&sess.client);
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(8);
    let (command, mut completion) = apply_command(
        next,
        Some((ConfigMode::Shared, "deepseek/new-shared".into())),
        false,
    );

    process_cmd(command, &mut sess, &evt_tx, 0).await;

    assert_eq!(completion.recv().await, Some(Ok(())));
    assert_eq!(sess.config.model, "openai/shared");
    assert_eq!(sess.effective_model(), "deepseek/new-shared");
    assert_eq!(Arc::as_ptr(&sess.client), original_client);
    assert_eq!(
        store
            .get_session("shared-command")
            .await
            .unwrap()
            .unwrap()
            .model
            .as_deref(),
        Some("openai/shared")
    );
    assert_eq!(
        store
            .get_session_model_overrides("shared-command")
            .await
            .unwrap()
            .unwrap()
            .shared
            .as_deref(),
        Some("deepseek/new-shared")
    );
    assert!(matches!(
        evt_rx.recv().await,
        Some(UiEvent::Session(SessionEvent::ModelSwitch(model))) if model == "new-shared"
    ));
}

#[tokio::test]
async fn provider_only_command_rebuilds_without_reinstalling_stale_outer_model() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let mut original = config("openai/shared", "openai/act");
    original.providers.insert(
        "openai".into(),
        ProviderConfig {
            base_url: "https://example.invalid/v1".into(),
            api_key: Some("test-placeholder".into()),
            ..Default::default()
        },
    );
    let mut next = original.clone();
    next.model = "outer/stale".into();
    next.providers.get_mut("openai").unwrap().base_url = "https://example.invalid/v2".into();
    let mut sess = session("provider-command", original, store);
    let original_client = Arc::as_ptr(&sess.client);
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(8);
    let (command, mut completion) = apply_command(next, None, true);

    process_cmd(command, &mut sess, &evt_tx, 0).await;

    assert_eq!(completion.recv().await, Some(Ok(())));
    assert_ne!(Arc::as_ptr(&sess.client), original_client);
    assert_eq!(sess.config.model, "openai/shared");
    assert_eq!(
        sess.config.providers["openai"].base_url,
        "https://example.invalid/v2"
    );
    assert!(evt_rx.try_recv().is_err());
}

#[tokio::test]
async fn env_reload_clears_durable_overrides_and_preserves_one_shot_skill_state() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let initial = config("openai/shared", "openai/act");
    let mut sess = session("env-command", initial.clone(), store.clone());
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(8);
    let (select, mut selected) = apply_command(
        initial,
        Some((ConfigMode::Act, "deepseek/session-act".into())),
        false,
    );
    process_cmd(select, &mut sess, &evt_tx, 0).await;
    assert_eq!(selected.recv().await, Some(Ok(())));
    assert!(matches!(evt_rx.recv().await, Some(UiEvent::Session(_))));
    sess.set_skill(Some("generic instructions".into()));
    sess.set_skill_body_delivered(true);
    let (reload, mut reloaded) = env_command(config("openai/env-shared", "openai/env-act"));

    process_cmd(reload, &mut sess, &evt_tx, 0).await;

    assert_eq!(reloaded.recv().await, Some(Ok(())));
    assert_eq!(sess.config.model, "openai/env-shared");
    assert_eq!(sess.effective_model(), "openai/env-act");
    assert_eq!(sess.effective_context_limit(), 15_000);
    assert_eq!(sess.model_override(ConfigMode::Act), None);
    assert_eq!(
        store
            .get_session_model_overrides("env-command")
            .await
            .unwrap(),
        Some(SessionModelOverrides::default())
    );
    assert_eq!(
        sess.skill_prompt_cloned().as_deref(),
        Some("generic instructions")
    );
    assert!(sess.skill_body_delivered());
    assert!(matches!(
        evt_rx.recv().await,
        Some(UiEvent::Session(SessionEvent::ModelSwitch(model))) if model == "env-act"
    ));
}

/// The mid-turn path: `ApplySessionDeferred` carries no ack channel. The
/// serial worker applies it at the turn boundary exactly like `ApplySession`
/// (override persisted, effective model switched, client retained) and then
/// reports through `ModelCommit` — ordered after the `ModelSwitch` event on
/// the same channel, so the UI fold commits the display only after the
/// transcript saw the switch.
#[tokio::test]
async fn deferred_session_command_reports_commit_after_model_switch() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let next = config("openai/shared", "openai/act");
    let mut sess = session("model-deferred", next.clone(), store.clone());
    let original_client = Arc::as_ptr(&sess.client);
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(8);
    let selection = Some((ConfigMode::Act, "deepseek/session-act".into()));

    let quit = process_cmd(
        UiCmd::ApplySessionDeferred {
            config: Box::new(next),
            selection: selection.clone(),
        },
        &mut sess,
        &evt_tx,
        0,
    )
    .await;

    assert!(!quit);
    assert!(matches!(
        evt_rx.recv().await,
        Some(UiEvent::Session(SessionEvent::ModelSwitch(model))) if model == "session-act"
    ));
    assert!(matches!(
        evt_rx.recv().await,
        Some(UiEvent::ModelCommit {
            selection: reported,
            result: Ok(()),
        }) if reported == selection
    ));
    assert!(
        evt_rx.try_recv().is_err(),
        "exactly ModelSwitch then ModelCommit"
    );
    assert_eq!(sess.config.model, "openai/shared");
    assert_eq!(sess.effective_model(), "deepseek/session-act");
    assert_eq!(Arc::as_ptr(&sess.client), original_client);
    assert_eq!(
        store
            .get_session_model_overrides("model-deferred")
            .await
            .unwrap()
            .unwrap()
            .act
            .as_deref(),
        Some("deepseek/session-act")
    );
}

/// Re-selecting the already-effective model emits no `ModelSwitch`, but the
/// commit still lands so the UI can settle the queued pick's marker.
#[tokio::test]
async fn deferred_session_command_without_effective_change_reports_commit_only() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let next = config("openai/shared", "openai/act");
    let mut sess = session("model-deferred-unchanged", next.clone(), store.clone());
    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(8);
    let selection = Some((ConfigMode::Act, "openai/act".into()));

    process_cmd(
        UiCmd::ApplySessionDeferred {
            config: Box::new(next),
            selection: selection.clone(),
        },
        &mut sess,
        &evt_tx,
        0,
    )
    .await;

    assert!(matches!(
        evt_rx.recv().await,
        Some(UiEvent::ModelCommit {
            selection: reported,
            result: Ok(()),
        }) if reported == selection
    ));
    assert!(evt_rx.try_recv().is_err(), "no ModelSwitch");
    assert_eq!(sess.effective_model(), "openai/act");
}

use std::{path::Path, sync::Arc};

use opencoder_core::{resolve_agent, Config, ConfigMode, ModeConfig, ModesConfig};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_session::{model::effective_config_for, resume, SessionState};
use opencoder_store::{SessionMeta, SessionModelOverrides, Store, TursoStore};

use crate::app_helpers::{preferred_explicit_model, reapply_session_model};

fn config() -> Config {
    Config {
        model: "provider/file".into(),
        modes: ModesConfig {
            shared: ModeConfig {
                model: Some("provider/shared".into()),
                max_tokens: Some(100),
                ..Default::default()
            },
            act: Some(ModeConfig {
                model: Some("provider/act".into()),
                reasoning_effort: Some("high".into()),
                max_tokens: Some(200),
                context_limit: Some(6_000),
                ..Default::default()
            }),
            plan: Some(ModeConfig {
                model: Some("provider/plan".into()),
                max_tokens: Some(300),
                context_limit: Some(5_000),
                ..Default::default()
            }),
        },
        ..Default::default()
    }
}

fn meta(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        agent: Some("act".into()),
        model: Some("provider/stored".into()),
        created_at: 1,
        updated_at: 1,
        ..Default::default()
    }
}

fn client() -> Arc<dyn ChatStream> {
    Arc::new(MockChatClient::new())
}

async fn restore(store: Arc<TursoStore>, id: &str, config: Config, dir: &Path) -> SessionState {
    resume(store, id, config, client(), dir.to_path_buf())
        .await
        .unwrap()
}

#[tokio::test]
async fn resume_cli_model_beats_environment_and_persists_empty_mode_metadata() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    store
        .create_session_with_model_overrides(
            &meta("tui-explicit"),
            Some(&SessionModelOverrides {
                act: Some("provider/menu-act".into()),
                plan: Some("provider/menu-plan".into()),
                ..Default::default()
            }),
        )
        .await
        .unwrap();

    let mut startup = config();
    startup.apply_explicit_model("env/model");
    startup.apply_explicit_model("cli/model");
    let explicit = preferred_explicit_model(Some("cli/model"), Some("env/model"));
    let mut session = restore(store.clone(), "tui-explicit", startup, dir.path()).await;
    assert_eq!(session.effective_model(), "provider/menu-act");

    assert_eq!(
        reapply_session_model(&mut session, &explicit)
            .await
            .unwrap()
            .as_deref(),
        Some("cli/model")
    );
    for mode in [ConfigMode::Shared, ConfigMode::Act, ConfigMode::Plan] {
        let agent = resolve_agent(match mode {
            ConfigMode::Shared => "build",
            ConfigMode::Act => "act",
            ConfigMode::Plan => "plan",
        })
        .unwrap();
        let effective = effective_config_for(&session.config, &agent, session.model_overrides());
        assert_eq!(effective.model, "cli/model");
    }
    let act = effective_config_for(
        &session.config,
        &resolve_agent("act").unwrap(),
        session.model_overrides(),
    );
    assert_eq!(act.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(act.max_tokens, Some(200));
    assert_eq!(act.context_limit(), 6_000);
    assert_eq!(
        store
            .get_session_model_overrides("tui-explicit")
            .await
            .unwrap(),
        Some(SessionModelOverrides::default())
    );
    assert_eq!(
        store
            .get_session("tui-explicit")
            .await
            .unwrap()
            .unwrap()
            .model
            .as_deref(),
        Some("cli/model")
    );
}

#[tokio::test]
async fn resume_environment_model_wins_without_cli_and_preserves_legacy_null() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    store.create_session(&meta("tui-env")).await.unwrap();
    let mut startup = config();
    startup.apply_explicit_model("env/model");
    let explicit = preferred_explicit_model(None, Some("env/model"));
    let mut session = restore(store.clone(), "tui-env", startup, dir.path()).await;

    reapply_session_model(&mut session, &explicit)
        .await
        .unwrap();
    assert_eq!(session.effective_model(), "env/model");
    assert_eq!(session.model_overrides(), None);
    assert_eq!(
        store.get_session_model_overrides("tui-env").await.unwrap(),
        None
    );
    assert_eq!(
        store
            .get_session("tui-env")
            .await
            .unwrap()
            .unwrap()
            .model
            .as_deref(),
        Some("env/model")
    );
}

#[tokio::test]
async fn fresh_explicit_model_remains_lazy_and_mode_aware() {
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "tui-lazy",
        resolve_agent("act").unwrap(),
        config(),
        client(),
        dir.path().to_path_buf(),
    )
    .with_store(store.clone());
    let explicit = preferred_explicit_model(Some("cli/model"), Some("env/model"));

    reapply_session_model(&mut session, &explicit)
        .await
        .unwrap();
    assert_eq!(session.effective_model(), "cli/model");
    assert_eq!(
        session.model_overrides(),
        Some(&SessionModelOverrides::default())
    );
    assert!(store.get_session("tui-lazy").await.unwrap().is_none());
}

use std::{path::Path, sync::Arc};

use opencoder_core::{resolve_agent, Config, ConfigMode, ModeConfig, ModesConfig};
use opencoder_llm::{ChatStream, MockChatClient};
use opencoder_session::{fork::fork_session_with_id, resume, SessionState};
use opencoder_store::{SessionMeta, SessionModelOverrides, Store, TursoStore};

#[path = "mode_model_persistence/explicit_env.rs"]
mod explicit_env;

#[path = "mode_model_persistence/lazy.rs"]
mod lazy;

#[path = "mode_model_persistence/shared.rs"]
mod shared;

fn config(shared: &str, act: &str, plan: &str) -> Config {
    Config {
        model: shared.into(),
        modes: ModesConfig {
            act: Some(ModeConfig {
                model: Some(act.into()),
                context_limit: Some(6_000),
                ..Default::default()
            }),
            plan: Some(ModeConfig {
                model: Some(plan.into()),
                context_limit: Some(5_000),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn client() -> Arc<dyn ChatStream> {
    Arc::new(MockChatClient::new())
}

fn state(id: &str, agent: &str, config: Config, store: Arc<dyn Store>, dir: &Path) -> SessionState {
    SessionState::new(
        id,
        resolve_agent(agent).unwrap(),
        config,
        client(),
        dir.to_path_buf(),
    )
    .with_store(store)
}

fn meta(id: &str, model: &str) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        agent: Some("act".into()),
        model: Some(model.into()),
        created_at: 1,
        updated_at: 1,
        ..Default::default()
    }
}

#[tokio::test]
async fn overrides_survive_resume_and_normal_reload_without_config_writes() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let original = config("provider/shared", "provider/act", "provider/plan");
    let mut session = state("persist", "act", original, store.clone(), dir.path());

    session
        .set_mode_model_override(ConfigMode::Act, "provider/session-act")
        .await
        .unwrap();
    session
        .set_mode_model_override(ConfigMode::Plan, "provider/session-plan")
        .await
        .unwrap();
    assert_eq!(session.config.model, "provider/shared");
    assert_eq!(
        store
            .get_session("persist")
            .await
            .unwrap()
            .unwrap()
            .model
            .as_deref(),
        Some("provider/shared")
    );

    let reloaded = config("provider/reloaded", "provider/new-act", "provider/new-plan");
    session.apply_config_reload(reloaded.clone(), client());
    assert_eq!(session.effective_model(), "provider/session-act");
    assert_eq!(
        session.model_override(ConfigMode::Plan),
        Some("provider/session-plan")
    );

    let mut restored = resume(
        store.clone(),
        "persist",
        reloaded.clone(),
        client(),
        dir.path().into(),
    )
    .await
    .unwrap();
    assert_eq!(restored.effective_model(), "provider/session-act");
    restored.agent = resolve_agent("plan").unwrap();
    assert_eq!(restored.effective_model(), "provider/session-plan");

    restored
        .clear_mode_model_override(ConfigMode::Plan)
        .await
        .unwrap();
    assert_eq!(restored.effective_model(), "provider/new-plan");
    assert_eq!(
        restored.model_override(ConfigMode::Act),
        Some("provider/session-act")
    );
    assert_eq!(
        store.get_session_model_overrides("persist").await.unwrap(),
        Some(SessionModelOverrides {
            act: Some("provider/session-act".into()),
            plan: None,
            shared: None,
        })
    );

    let target = config(
        "provider/target",
        "provider/target-act",
        "provider/target-plan",
    );
    restored
        .apply_env_reload(target.clone(), Some(client()))
        .await
        .unwrap();
    assert_eq!(
        restored.model_overrides(),
        Some(&SessionModelOverrides::default())
    );
    assert_eq!(restored.effective_model(), "provider/target-plan");
    assert_eq!(
        store.get_session_model_overrides("persist").await.unwrap(),
        Some(SessionModelOverrides::default())
    );
    assert_eq!(
        store
            .get_session("persist")
            .await
            .unwrap()
            .unwrap()
            .model
            .as_deref(),
        Some("provider/target"),
        "Env reload persists Shared beside the cleared Act/Plan metadata"
    );
    restored.agent = resolve_agent("build").unwrap();
    assert_eq!(restored.effective_model(), "provider/target");

    let resumed = resume(store, "persist", target, client(), dir.path().into())
        .await
        .unwrap();
    assert_eq!(resumed.effective_model(), "provider/target-act");
    let mut shared = resumed;
    shared.agent = resolve_agent("build").unwrap();
    assert_eq!(shared.effective_model(), "provider/target");
}

#[tokio::test]
async fn resume_distinguishes_legacy_null_from_new_empty_metadata() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    store
        .create_session(&meta("legacy", "provider/legacy"))
        .await
        .unwrap();
    store
        .create_session_with_model_overrides(
            &meta("fresh", "provider/shared"),
            Some(&SessionModelOverrides::default()),
        )
        .await
        .unwrap();
    let cfg = config("provider/default", "provider/act", "provider/plan");
    let dir = tempfile::tempdir().unwrap();

    let legacy = resume(
        store.clone(),
        "legacy",
        cfg.clone(),
        client(),
        dir.path().into(),
    )
    .await
    .unwrap();
    assert_eq!(legacy.model_overrides(), None);
    assert_eq!(legacy.effective_model(), "provider/legacy");
    assert_eq!(
        legacy.effective_context_limit(),
        opencoder_core::DEFAULT_CONTEXT_LIMIT
    );

    let fresh = resume(store, "fresh", cfg, client(), dir.path().into())
        .await
        .unwrap();
    assert_eq!(
        fresh.model_overrides(),
        Some(&SessionModelOverrides::default())
    );
    assert_eq!(fresh.effective_model(), "provider/act");
    assert_eq!(fresh.effective_context_limit(), 6_000);
}

#[tokio::test]
async fn explicit_model_after_resume_clears_mode_precedence_and_labels() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let stored = SessionModelOverrides {
        act: Some("provider/stored-act".into()),
        plan: Some("provider/stored-plan".into()),
        shared: None,
    };
    store
        .create_session_with_model_overrides(
            &meta("explicit-resume", "provider/legacy"),
            Some(&stored),
        )
        .await
        .unwrap();
    let mut session = resume(
        store.clone(),
        "explicit-resume",
        config("provider/shared", "provider/act", "provider/plan"),
        client(),
        dir.path().into(),
    )
    .await
    .unwrap();
    assert_eq!(session.effective_model(), "provider/stored-act");

    assert!(session.apply_explicit_model("override/explicit-model"));
    assert_eq!(
        session.model_overrides(),
        Some(&SessionModelOverrides::default()),
        "mode-aware rows stay mode-aware with empty Act/Plan selections"
    );
    assert_eq!(session.effective_model(), "override/explicit-model");
    assert_eq!(session.current_model(), "explicit-model");
    assert!(session.take_model_switch().is_none());
    assert!(session.config.modes.shared.model.is_none());
    assert!(session.config.modes.act.as_ref().unwrap().model.is_none());
    assert!(session.config.modes.plan.as_ref().unwrap().model.is_none());
    assert_eq!(
        session.config.modes.act.as_ref().unwrap().context_limit,
        Some(6_000),
        "the explicit model does not clear generation overlays"
    );
    assert_eq!(
        store
            .get_session_model_overrides("explicit-resume")
            .await
            .unwrap(),
        Some(stored)
    );
    assert!(!session.apply_explicit_model("override/explicit-model"));
}

#[tokio::test]
async fn explicit_model_is_lazy_until_a_later_menu_selection() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let mut session = state(
        "explicit-lazy",
        "act",
        config("provider/shared", "provider/act", "provider/plan"),
        store.clone(),
        dir.path(),
    );
    session.inherit_model_overrides(Some(&SessionModelOverrides {
        act: Some("provider/stored-act".into()),
        plan: Some("provider/stored-plan".into()),
        shared: None,
    }));

    assert!(session.apply_explicit_model("override/explicit-model"));
    assert!(store.get_session("explicit-lazy").await.unwrap().is_none());

    session
        .set_mode_model_override(ConfigMode::Plan, "provider/menu-plan")
        .await
        .unwrap();
    session.agent = resolve_agent("plan").unwrap();
    assert_eq!(session.effective_model(), "provider/menu-plan");
    assert_eq!(
        store
            .get_session_model_overrides("explicit-lazy")
            .await
            .unwrap(),
        Some(SessionModelOverrides {
            act: None,
            plan: Some("provider/menu-plan".into()),
            shared: None,
        }),
        "a later selection edits only its mode; the other keeps inheriting the explicit model"
    );
}

#[tokio::test]
async fn inheritance_and_fork_copy_per_mode_selections() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let cfg = config("provider/shared", "provider/act", "provider/plan");
    let mut source = state("source", "act", cfg.clone(), store.clone(), dir.path());
    source
        .set_mode_model_override(ConfigMode::Act, "provider/inherited-act")
        .await
        .unwrap();
    source
        .set_mode_model_override(ConfigMode::Plan, "provider/inherited-plan")
        .await
        .unwrap();

    let mut child = state("memory-child", "plan", cfg, store.clone(), dir.path());
    child.inherit_model_overrides_from(&source);
    assert_eq!(child.effective_model(), "provider/inherited-plan");

    fork_session_with_id(store.as_ref(), "source", "forked")
        .await
        .unwrap();
    assert_eq!(
        store.get_session_model_overrides("forked").await.unwrap(),
        source.model_overrides().cloned()
    );
    let forked = resume(
        store,
        "forked",
        config("x/shared", "x/act", "x/plan"),
        client(),
        dir.path().into(),
    )
    .await
    .unwrap();
    assert_eq!(forked.effective_model(), "provider/inherited-act");
}

#[tokio::test]
async fn failed_override_save_rolls_back_memory_and_keeps_old_row() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let cfg = config("provider/shared", "provider/act", "provider/plan");
    let mut session = state("rollback", "act", cfg, store.clone(), dir.path());
    session
        .set_mode_model_override(ConfigMode::Act, "provider/original")
        .await
        .unwrap();

    let conn = store.conn().await.unwrap();
    conn.execute(
        "CREATE TRIGGER reject_model_override BEFORE UPDATE OF model_overrides_json ON sessions BEGIN SELECT RAISE(FAIL, 'injected model metadata failure'); END",
        (),
    )
    .await
    .unwrap();
    session
        .set_mode_model_override(ConfigMode::Act, "provider/rejected")
        .await
        .unwrap_err();
    assert_eq!(
        session.model_override(ConfigMode::Act),
        Some("provider/original")
    );
    assert_eq!(session.effective_model(), "provider/original");
    conn.execute("DROP TRIGGER reject_model_override", ())
        .await
        .unwrap();
    assert_eq!(
        store
            .get_session_model_overrides("rollback")
            .await
            .unwrap()
            .unwrap()
            .act
            .as_deref(),
        Some("provider/original")
    );
}

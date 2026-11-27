use super::*;

fn shared_config(base: &str, shared: &str, context: u64) -> Config {
    let mut config = config(base, "provider/config-act", "provider/config-plan");
    config.modes.shared = ModeConfig {
        model: Some(shared.into()),
        max_tokens: Some(321),
        context_limit: Some(context),
        ..Default::default()
    };
    config
}

#[tokio::test]
async fn shared_selection_survives_resume_fork_and_new_task_inheritance() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let original = shared_config("provider/base", "provider/config-shared", 6_400);
    let mut session = state(
        "shared-source",
        "build",
        original,
        store.clone(),
        dir.path(),
    );
    assert_eq!(session.effective_model(), "provider/config-shared");

    session
        .set_shared_model_override("provider/session-shared")
        .await
        .unwrap();
    assert_eq!(session.config.model, "provider/base");
    assert_eq!(
        session.config.modes.shared.model.as_deref(),
        Some("provider/config-shared")
    );
    assert_eq!(session.effective_model(), "provider/session-shared");
    assert_eq!(session.effective_config().max_tokens, Some(321));
    assert_eq!(session.effective_context_limit(), 6_400);
    assert_eq!(
        store
            .get_session("shared-source")
            .await
            .unwrap()
            .unwrap()
            .model
            .as_deref(),
        Some("provider/base")
    );
    assert_eq!(
        session.model_override(ConfigMode::Shared),
        Some("provider/session-shared")
    );

    let child = {
        let mut child = state(
            "shared-new-task",
            "build",
            shared_config("task/base", "task/config-shared", 5_000),
            store.clone(),
            dir.path(),
        );
        child.inherit_model_overrides_from(&session);
        child
    };
    assert_eq!(child.effective_model(), "provider/session-shared");
    assert!(store
        .get_session("shared-new-task")
        .await
        .unwrap()
        .is_none());

    let restored = resume(
        store.clone(),
        "shared-source",
        shared_config("resume/base", "resume/config-shared", 7_000),
        client(),
        dir.path().into(),
    )
    .await
    .unwrap();
    assert_eq!(restored.effective_model(), "provider/session-shared");
    assert_eq!(restored.effective_context_limit(), 7_000);

    fork_session_with_id(store.as_ref(), "shared-source", "shared-fork")
        .await
        .unwrap();
    let forked = resume(
        store.clone(),
        "shared-fork",
        shared_config("fork/base", "fork/config-shared", 8_000),
        client(),
        dir.path().into(),
    )
    .await
    .unwrap();
    assert_eq!(forked.effective_model(), "provider/session-shared");
    assert_eq!(
        store
            .get_session_model_overrides("shared-fork")
            .await
            .unwrap(),
        session.model_overrides().cloned()
    );

    let mut cleared = restored;
    cleared.clear_shared_model_override().await.unwrap();
    assert_eq!(cleared.effective_model(), "resume/config-shared");
    assert_eq!(cleared.model_override(ConfigMode::Shared), None);
}

#[tokio::test]
async fn shared_base_save_env_reload_and_explicit_clear_keep_effective_overlays() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let mut session = state(
        "shared-reload",
        "build",
        shared_config("provider/base", "provider/config-shared", 6_400),
        store.clone(),
        dir.path(),
    );
    session
        .set_mode_model_override(ConfigMode::Shared, "provider/session-shared")
        .await
        .unwrap();

    session
        .set_shared_model("provider/disk-base")
        .await
        .unwrap();
    assert_eq!(session.config.model, "provider/disk-base");
    assert_eq!(session.effective_model(), "provider/session-shared");
    assert_eq!(session.effective_config().model, "provider/session-shared");

    let target = shared_config("target/base", "target/config-shared", 7_200);
    session
        .apply_env_reload(target.clone(), Some(client()))
        .await
        .unwrap();
    assert_eq!(
        session.model_overrides(),
        Some(&SessionModelOverrides::default())
    );
    assert_eq!(session.effective_model(), "target/config-shared");
    assert_eq!(session.effective_context_limit(), 7_200);
    assert_eq!(
        store
            .get_session("shared-reload")
            .await
            .unwrap()
            .unwrap()
            .model
            .as_deref(),
        Some("target/base")
    );

    session
        .set_shared_model_override("provider/temporary-shared")
        .await
        .unwrap();
    session
        .apply_explicit_model_and_persist("override/explicit")
        .await
        .unwrap();
    assert_eq!(
        session.model_overrides(),
        Some(&SessionModelOverrides::default())
    );
    assert_eq!(session.effective_model(), "override/explicit");
    assert_eq!(session.effective_context_limit(), 7_200);
    assert_eq!(session.effective_config().max_tokens, Some(321));

    let mut explicit_config = target.clone();
    explicit_config.apply_explicit_model("override/explicit");
    let resumed = resume(
        store,
        "shared-reload",
        explicit_config,
        client(),
        dir.path().into(),
    )
    .await
    .unwrap();
    assert_eq!(resumed.effective_model(), "override/explicit");
    assert_eq!(resumed.effective_context_limit(), 7_200);
}

#[tokio::test]
async fn selecting_shared_on_legacy_row_pins_other_primary_modes() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    store
        .create_session(&meta("shared-legacy", "provider/legacy"))
        .await
        .unwrap();
    let mut session = resume(
        store,
        "shared-legacy",
        shared_config("provider/config-base", "provider/config-shared", 6_000),
        client(),
        dir.path().into(),
    )
    .await
    .unwrap();
    session
        .set_shared_model_override("provider/session-shared")
        .await
        .unwrap();
    let stored = session.model_overrides().unwrap();
    assert_eq!(stored.shared.as_deref(), Some("provider/session-shared"));
    assert_eq!(stored.act.as_deref(), Some("provider/legacy"));
    assert_eq!(stored.plan.as_deref(), Some("provider/legacy"));
}

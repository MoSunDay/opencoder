use super::*;

#[tokio::test]
async fn persisted_explicit_model_writes_empty_object_or_legacy_null() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let cfg = config("provider/shared", "provider/act", "provider/plan");
    store
        .create_session_with_model_overrides(
            &meta("mode-row", "provider/stored"),
            Some(&SessionModelOverrides {
                act: Some("provider/old-act".into()),
                plan: Some("provider/old-plan".into()),
                shared: None,
            }),
        )
        .await
        .unwrap();
    store
        .create_session(&meta("legacy-row", "provider/legacy"))
        .await
        .unwrap();

    let mut mode = resume(
        store.clone(),
        "mode-row",
        cfg.clone(),
        client(),
        dir.path().into(),
    )
    .await
    .unwrap();
    assert!(mode
        .apply_explicit_model_and_persist("override/explicit-model")
        .await
        .unwrap());
    assert_eq!(
        store.get_session_model_overrides("mode-row").await.unwrap(),
        Some(SessionModelOverrides::default())
    );
    assert_eq!(
        store
            .get_session("mode-row")
            .await
            .unwrap()
            .unwrap()
            .model
            .as_deref(),
        Some("override/explicit-model")
    );
    assert_eq!(mode.effective_context_limit(), 6_000);
    let mut explicit_config = cfg.clone();
    explicit_config.apply_explicit_model("override/explicit-model");
    let restored = resume(
        store.clone(),
        "mode-row",
        explicit_config,
        client(),
        dir.path().into(),
    )
    .await
    .unwrap();
    assert_eq!(restored.effective_model(), "override/explicit-model");
    assert_eq!(restored.effective_context_limit(), 6_000);

    let mut legacy = resume(
        store.clone(),
        "legacy-row",
        cfg,
        client(),
        dir.path().into(),
    )
    .await
    .unwrap();
    legacy
        .apply_explicit_model_and_persist("override/legacy-explicit")
        .await
        .unwrap();
    assert_eq!(legacy.model_overrides(), None);
    assert_eq!(
        store
            .get_session_model_overrides("legacy-row")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .get_session("legacy-row")
            .await
            .unwrap()
            .unwrap()
            .model
            .as_deref(),
        Some("override/legacy-explicit")
    );
}

#[tokio::test]
async fn lazy_env_reload_materializes_shared_and_empty_metadata_together() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let mut session = state(
        "lazy-env",
        "act",
        config("provider/old", "provider/old-act", "provider/old-plan"),
        store.clone(),
        dir.path(),
    );
    let target = config(
        "provider/target",
        "provider/target-act",
        "provider/target-plan",
    );
    session
        .apply_env_reload(target, Some(client()))
        .await
        .unwrap();
    assert!(store.get_session("lazy-env").await.unwrap().is_none());

    session.set_shared_model("provider/target").await.unwrap();
    assert_eq!(
        store
            .get_session("lazy-env")
            .await
            .unwrap()
            .unwrap()
            .model
            .as_deref(),
        Some("provider/target")
    );
    assert_eq!(
        store.get_session_model_overrides("lazy-env").await.unwrap(),
        Some(SessionModelOverrides::default())
    );
}

#[tokio::test]
async fn failed_model_metadata_save_rolls_back_explicit_and_env_changes() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    let dir = tempfile::tempdir().unwrap();
    let original = SessionModelOverrides {
        act: Some("provider/original-act".into()),
        plan: Some("provider/original-plan".into()),
        shared: None,
    };
    store
        .create_session_with_model_overrides(
            &meta("atomic-rollback", "provider/stored"),
            Some(&original),
        )
        .await
        .unwrap();
    let cfg = config("provider/shared", "provider/act", "provider/plan");
    let mut session = resume(
        store.clone(),
        "atomic-rollback",
        cfg,
        client(),
        dir.path().into(),
    )
    .await
    .unwrap();
    let original_config = session.config.clone();
    let original_label = session.current_model().to_string();
    let conn = store.conn().await.unwrap();
    conn.execute(
        "CREATE TRIGGER reject_model_override BEFORE UPDATE OF model_overrides_json ON sessions BEGIN SELECT RAISE(FAIL, 'injected model metadata failure'); END",
        (),
    )
    .await
    .unwrap();

    session
        .apply_explicit_model_and_persist("override/rejected")
        .await
        .unwrap_err();
    assert_eq!(session.config.model, original_config.model);
    assert_eq!(session.config.modes, original_config.modes);
    assert_eq!(session.model_overrides(), Some(&original));
    assert_eq!(session.current_model(), original_label);
    assert_eq!(session.effective_model(), "provider/original-act");

    session
        .apply_env_reload(
            config(
                "provider/target",
                "provider/target-act",
                "provider/target-plan",
            ),
            Some(client()),
        )
        .await
        .unwrap_err();
    assert_eq!(session.config.model, original_config.model);
    assert_eq!(session.config.modes, original_config.modes);
    assert_eq!(session.model_overrides(), Some(&original));
    assert_eq!(session.effective_model(), "provider/original-act");
    assert_eq!(
        store
            .get_session("atomic-rollback")
            .await
            .unwrap()
            .unwrap()
            .model
            .as_deref(),
        Some("provider/stored"),
        "the compensating update restores the first durable field"
    );
    assert_eq!(
        store
            .get_session_model_overrides("atomic-rollback")
            .await
            .unwrap(),
        Some(original)
    );
    conn.execute("DROP TRIGGER reject_model_override", ())
        .await
        .unwrap();
}

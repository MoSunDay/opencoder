use super::*;

#[tokio::test]
async fn resume_then_new_preserves_shared_for_legacy_and_mode_aware_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::scoped_config_home(dir.path().join("config-home"));
    let store: Arc<dyn Store> = Arc::new(opencoder_store::TursoStore::open_memory().await.unwrap());
    let mut client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let mut config = Config {
        model: "outer/stale-shared".into(),
        ..Default::default()
    };
    config.provider.api_key = Some("test-key".into());
    std::fs::write(
        dir.path().join("opencoder.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    store
        .create_session(&opencoder_store::SessionMeta {
            id: "resumed".into(),
            agent: Some("act".into()),
            model: Some("resumed/shared".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let mode_overrides = opencoder_store::SessionModelOverrides {
        shared: Some("shared/session-override".into()),
        act: Some("act/session-override".into()),
        plan: None,
    };
    store
        .create_session_with_model_overrides(
            &opencoder_store::SessionMeta {
                id: "mode-aware".into(),
                agent: Some("act".into()),
                model: Some("resumed/mode-shared".into()),
                ..Default::default()
            },
            Some(&mode_overrides),
        )
        .await
        .unwrap();

    let initial = SessionState::new(
        "initial",
        resolve_agent("act").unwrap(),
        config.clone(),
        client.clone(),
        dir.path().to_path_buf(),
    );
    let mut session_id = initial.id.clone();
    let mut chat = ChatView {
        agent: "act".into(),
        model: config.model.clone(),
        shared_model: config.model.clone(),
        ..Default::default()
    };
    let (mut cmd_tx, _old_commands) = mpsc::channel(8);
    let (ui_tx, mut evt_rx) = mpsc::channel(16);
    let mut cancel = CancellationToken::new();
    let mut turn_cancel = Arc::new(Mutex::new(CancellationToken::new()));
    let mut children = ChildRuntimeHandles::from_session(&initial);
    let mut skill_handle = initial.skill_prompt.clone();
    let mut hub = initial.question_hub.clone();
    let mut sidecar = crate::sidecar_ui::spawn_actor(&initial, ui_tx, Some(store.clone()));
    let mut snapshots = std::collections::HashMap::new();
    let mut running = false;
    let mut history = Vec::new();
    let mut scroll = 0;
    let mut follow = true;
    let mut queue_scroll = 0;
    let mut sys = 0;
    let mut queue = Vec::new();
    let mut skill = None;
    let mut body = None;
    let mut input = String::new();
    let mut cursor = 0;
    let mut hist_idx = None;

    macro_rules! switch_to {
        ($pick:expr) => {
            let state = switch_session(
                $pick,
                &TuiOpts::default(),
                &None,
                &mut cmd_tx,
                &mut evt_rx,
                dir.path(),
                &config,
                &client,
                &store,
                &mut snapshots,
                &mut running,
                &mut chat,
                &mut history,
                &mut scroll,
                &mut follow,
                &mut queue_scroll,
                &mut sys,
                &mut queue,
                &mut skill,
                &mut body,
                &mut session_id,
                &mut input,
                &mut cursor,
                &mut hist_idx,
                &mut cancel,
                &mut turn_cancel,
                &mut children,
                &mut skill_handle,
                &mut hub,
                &mut sidecar,
            )
            .await
            .unwrap();
            assert_eq!(state.shared_model, chat.shared_model);
            assert_eq!(state.model, chat.model);
            assert_eq!(state.model_overrides, chat.model_overrides);
            config = state.config;
            client = state.client;
        };
    }

    switch_to!(crate::task::TaskPick::Resume("resumed".into()));
    assert_eq!(chat.shared_model, "resumed/shared");
    assert_eq!(chat.model, "resumed/shared");
    assert_eq!(chat.model_overrides, None);

    switch_to!(crate::task::TaskPick::New);
    assert_ne!(session_id, "resumed");
    assert_eq!(
        chat.shared_model, "resumed/shared",
        "New must inherit the resumed session's Shared model, not the stale outer config"
    );
    assert_eq!(
        chat.model, "resumed/shared",
        "NULL metadata keeps legacy Act/Plan inheritance on the Shared model"
    );
    assert_eq!(chat.model_overrides, None);
    assert!(
        store.get_session(&session_id).await.unwrap().is_none(),
        "opening New remains lazy and does not persist inherited metadata"
    );

    switch_to!(crate::task::TaskPick::Resume("mode-aware".into()));
    assert_eq!(chat.shared_model, "shared/session-override");
    assert_eq!(chat.model, "act/session-override");
    assert_eq!(chat.model_overrides, Some(mode_overrides.clone()));

    switch_to!(crate::task::TaskPick::New);
    assert_ne!(session_id, "mode-aware");
    assert_eq!(
        chat.shared_model, "shared/session-override",
        "mode-aware New inherits Shared rather than the effective Act selection"
    );
    assert_eq!(chat.model, "act/session-override");
    assert_eq!(chat.model_overrides, Some(mode_overrides));
    assert_eq!(config.model, "shared/session-override");
    assert_eq!(
        client.backend(),
        "chat",
        "New installs the client built from its effective config"
    );
    assert!(
        store.get_session(&session_id).await.unwrap().is_none(),
        "mode-aware inheritance also remains lazy"
    );
    assert_eq!(
        Config::load(dir.path()).unwrap().model,
        "outer/stale-shared",
        "inheriting a session selection must not rewrite configuration"
    );
    cmd_tx.send(UiCmd::Quit).await.unwrap();
}

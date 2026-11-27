use super::*;
use opencoder_core::{resolve_agent, ModeConfig};
use opencoder_session::model::effective_config_for;
use opencoder_store::{SessionMeta, Store, TursoStore};

#[test]
fn legacy_conversion_matches_session_shared_and_other_mode_rules() {
    let config = config();
    let mut view = chat(&config);
    let shared = display_overrides(&view, &config, ConfigMode::Shared);
    assert_eq!(shared.shared.as_deref(), Some("openai/shared"));
    assert_eq!(shared.act.as_deref(), Some("openai/shared"));
    assert_eq!(shared.plan.as_deref(), Some("openai/shared"));

    let act = display_overrides(&view, &config, ConfigMode::Act);
    assert_eq!(act.shared.as_deref(), Some("openai/shared"));
    assert_eq!(act.act, None);
    assert_eq!(act.plan.as_deref(), Some("openai/shared"));

    let plan = display_overrides(&view, &config, ConfigMode::Plan);
    assert_eq!(plan.shared.as_deref(), Some("openai/shared"));
    assert_eq!(plan.act.as_deref(), Some("openai/shared"));
    assert_eq!(plan.plan, None);

    view.model_overrides = Some(SessionModelOverrides {
        shared: None,
        act: Some("pinned/act".into()),
        plan: None,
    });
    let next = display_overrides(&view, &config, ConfigMode::Plan);
    assert_eq!(next.act.as_deref(), Some("pinned/act"));
}

#[tokio::test]
async fn acknowledged_act_plan_conversion_aligns_ui_worker_and_stored_shared() {
    for (agent, mode, mode_key) in [
        ("act", ConfigMode::Act, "act"),
        ("plan", ConfigMode::Plan, "plan"),
    ] {
        let id = format!("legacy-{mode_key}");
        let store = Arc::new(TursoStore::open_memory().await.unwrap());
        let mut config = config();
        config.modes.shared = ModeConfig {
            model: Some("openai/config-shared".into()),
            max_tokens: Some(4_096),
            ..Default::default()
        };
        let legacy_base = config.model.clone();
        let selected = format!("openai/session-{mode_key}");
        store
            .create_session(&SessionMeta {
                id: id.clone(),
                agent: Some(agent.into()),
                model: Some(legacy_base.clone()),
                ..Default::default()
            })
            .await
            .unwrap();
        let mut worker = opencoder_session::resume::resume(
            store.clone(),
            &id,
            config.clone(),
            Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
            std::env::temp_dir(),
        )
        .await
        .unwrap();
        let mut outer_config = config.clone();
        let mut outer_client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
        let mut chat = ChatView {
            agent: agent.into(),
            shared_model: crate::model_scope::shared_model(&config, None),
            model: legacy_base.clone(),
            ..Default::default()
        };
        let (tx, mut rx) = mpsc::channel(2);
        let (evt_tx, _evt_rx) = mpsc::channel(8);
        let patch = serde_json::json!({
            "modes": {(mode_key): {"model": selected}}
        });

        let ((), ()) = tokio::join!(
            switch_session(patch, &mut outer_config, &mut outer_client, &tx, &mut chat),
            async {
                let command = rx.recv().await.expect("ApplySession command");
                assert!(matches!(
                    &command,
                    UiCmd::ApplySession {
                        selection: Some((selected_mode, _)),
                        ..
                    } if *selected_mode == mode
                ));
                assert!(!crate::worker::process_cmd(command, &mut worker, &evt_tx, 0).await);
            }
        );

        let ui_overrides = chat.model_overrides.clone().unwrap();
        let worker_overrides = worker.model_overrides().cloned().unwrap();
        assert_eq!(ui_overrides, worker_overrides);
        assert_eq!(
            store.get_session_model_overrides(&id).await.unwrap(),
            Some(worker_overrides.clone())
        );
        assert_eq!(
            worker_overrides.shared.as_deref(),
            Some(legacy_base.as_str())
        );
        assert_eq!(worker_overrides.get(mode), Some(selected.as_str()));
        let other_mode = match mode {
            ConfigMode::Act => ConfigMode::Plan,
            ConfigMode::Plan => ConfigMode::Act,
            ConfigMode::Shared => unreachable!(),
        };
        assert_eq!(worker_overrides.get(other_mode), Some(legacy_base.as_str()));

        assert_eq!(chat.shared_model, legacy_base);
        assert_eq!(chat.model, selected);
        assert_eq!(worker.effective_config().model, selected);
        let ui_shared = crate::model_scope::effective_for_mode(
            &outer_config,
            &chat.shared_model,
            chat.model_overrides.as_ref(),
            ConfigMode::Shared,
        )
        .model;
        let worker_shared = effective_config_for(
            &worker.config,
            &resolve_agent("build").unwrap(),
            worker.model_overrides(),
        )
        .model;
        assert_eq!(ui_shared, legacy_base);
        assert_eq!(worker_shared, ui_shared);
    }
}

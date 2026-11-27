use super::*;
use opencoder_core::{ModeConfig, ModesConfig};
use opencoder_store::{SessionMeta, SessionModelOverrides, TursoStore};

fn provider(protocol: &str, api_key: Option<&str>) -> ProviderConfig {
    ProviderConfig {
        protocol: protocol.into(),
        base_url: "http://127.0.0.1:9/v1".into(),
        api_key: api_key.map(str::to_string),
        ..Default::default()
    }
}

fn restored_config() -> Config {
    let mut config = Config {
        model: "legacy/base".into(),
        provider: provider("chat_completions", None),
        modes: ModesConfig {
            shared: ModeConfig {
                model: Some("shared/from-config".into()),
                ..Default::default()
            },
            act: Some(ModeConfig {
                model: Some("act/from-config".into()),
                max_tokens: Some(2_200),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    config
        .providers
        .insert("legacy".into(), provider("chat_completions", None));
    // A distinct wire protocol also makes the selected provider observable in
    // request_body, in addition to its credential permitting construction.
    config
        .providers
        .insert("active".into(), provider("responses", Some("test-key")));
    config
}

fn restored_overrides() -> SessionModelOverrides {
    SessionModelOverrides {
        shared: Some("shared/from-session".into()),
        act: Some("active/from-session".into()),
        plan: None,
    }
}

async fn assert_switch_uses_restored_active_provider(pick: crate::task::TaskPick, id: &str) {
    let dir = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::scoped_config_home(dir.path().join("config-home"));
    let disk = restored_config();
    std::fs::write(
        dir.path().join("opencoder.json"),
        serde_json::to_vec(&disk).unwrap(),
    )
    .unwrap();
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    store
        .create_session_with_model_overrides(
            &SessionMeta {
                id: id.into(),
                agent: Some("act".into()),
                model: Some("legacy/base".into()),
                ..Default::default()
            },
            Some(&restored_overrides()),
        )
        .await
        .unwrap();

    let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let initial = SessionState::new(
        "current",
        resolve_agent("act").unwrap(),
        disk.clone(),
        client.clone(),
        dir.path().to_path_buf(),
    );
    let mut session_id = initial.id.clone();
    let mut chat = ChatView {
        agent: "act".into(),
        model: "old/act".into(),
        shared_model: "old/shared".into(),
        ..Default::default()
    };
    let (mut cmd_tx, _old_commands) = mpsc::channel(8);
    let (ui_tx, mut evt_rx) = mpsc::channel(16);
    let mut cancel = CancellationToken::new();
    let old_cancel = cancel.clone();
    let mut turn_cancel = Arc::new(Mutex::new(CancellationToken::new()));
    let mut children = ChildRuntimeHandles::from_session(&initial);
    let mut skill_handle = initial.skill_prompt.clone();
    let mut hub = initial.question_hub.clone();
    let mut sidecar = crate::sidecar_ui::spawn_actor(&initial, ui_tx, Some(store.clone()));
    let mut snapshots = std::collections::HashMap::new();
    let mut running = true;
    let mut history = Vec::new();
    let (mut scroll, mut follow, mut queue_scroll, mut sys) = (0, true, 0, 0);
    let (mut queue, mut skill, mut body) = (Vec::new(), None, None);
    let (mut input, mut cursor, mut hist_idx) = (String::new(), 0, None);

    let state = switch_session(
        pick,
        &TuiOpts::default(),
        &None,
        &mut cmd_tx,
        &mut evt_rx,
        dir.path(),
        &disk,
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

    assert!(old_cancel.is_cancelled());
    assert_eq!(state.config.model, "legacy/base");
    assert_eq!(state.shared_model, "shared/from-session");
    assert_eq!(state.model, "active/from-session");
    assert_eq!(state.model_overrides, Some(restored_overrides()));
    assert!(!Arc::ptr_eq(&state.client, &client));
    let request = ChatRequest {
        purpose: opencoder_llm::RequestPurpose::Conversation,
        model: state.model.clone(),
        messages: Vec::new(),
        tools: Vec::new(),
        tool_choice: None,
        temperature: None,
        max_tokens: None,
        reasoning_effort: None,
        cache_salt: None,
    };
    let body = state.client.request_body(&request).unwrap();
    assert_eq!(body["model"], "from-session");
    assert!(body.get("input").is_some());
    assert!(body.get("messages").is_none());
    assert_eq!(chat.shared_model, state.shared_model);
    assert_eq!(chat.model, state.model);
    cmd_tx.send(UiCmd::Quit).await.unwrap();
}

#[tokio::test]
async fn resume_without_explicit_model_uses_restored_active_provider() {
    assert_switch_uses_restored_active_provider(
        crate::task::TaskPick::Resume("resume-source".into()),
        "resume-source",
    )
    .await;
}

#[tokio::test]
async fn fork_without_explicit_model_uses_restored_active_provider() {
    assert_switch_uses_restored_active_provider(
        crate::task::TaskPick::Fork("fork-source".into()),
        "fork-source",
    )
    .await;
}

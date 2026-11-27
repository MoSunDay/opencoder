use super::*;
use opencoder_core::{ModeConfig, ModesConfig};
use opencoder_llm::{ChatRequest, MockChatClient};
use opencoder_store::{SessionMeta, SessionModelOverrides, TursoStore};

fn provider() -> ProviderConfig {
    ProviderConfig {
        protocol: "chat_completions".into(),
        base_url: "http://127.0.0.1:9".into(),
        api_key: Some("test-key".into()),
        ..Default::default()
    }
}

fn disk_config() -> Config {
    let mut config = Config {
        model: "stored/config".into(),
        provider: provider(),
        modes: ModesConfig {
            shared: ModeConfig {
                model: Some("stored/config-shared".into()),
                max_tokens: Some(1_100),
                context_limit: Some(11_000),
                ..Default::default()
            },
            act: Some(ModeConfig {
                model: Some("stored/config-act".into()),
                reasoning_effort: Some("high".into()),
                max_tokens: Some(2_200),
                context_limit: Some(22_000),
                ..Default::default()
            }),
            plan: Some(ModeConfig {
                model: Some("stored/config-plan".into()),
                max_tokens: Some(3_300),
                context_limit: Some(33_000),
                ..Default::default()
            }),
        },
        ..Default::default()
    };
    config.compaction.context_threshold = 73_000;
    config.providers.insert("cli".into(), provider());
    config
}

fn stored_overrides() -> SessionModelOverrides {
    SessionModelOverrides {
        shared: Some("stored/session-shared".into()),
        act: Some("stored/session-act".into()),
        plan: Some("stored/session-plan".into()),
    }
}

async fn create_target(store: &Arc<dyn Store>, id: &str) {
    store
        .create_session_with_model_overrides(
            &SessionMeta {
                id: id.into(),
                agent: Some("act".into()),
                model: Some("stored/session-shared".into()),
                ..Default::default()
            },
            Some(&stored_overrides()),
        )
        .await
        .unwrap();
}

fn write_config(dir: &tempfile::TempDir, config: &Config) {
    std::fs::write(
        dir.path().join("opencoder.json"),
        serde_json::to_vec(config).unwrap(),
    )
    .unwrap();
}

async fn assert_original_row(store: &Arc<dyn Store>, id: &str) {
    assert_eq!(
        store
            .get_session(id)
            .await
            .unwrap()
            .unwrap()
            .model
            .as_deref(),
        Some("stored/session-shared")
    );
    assert_eq!(
        store.get_session_model_overrides(id).await.unwrap(),
        Some(stored_overrides())
    );
}

async fn assert_explicit_row(store: &Arc<dyn Store>, id: &str) {
    assert_eq!(
        store
            .get_session(id)
            .await
            .unwrap()
            .unwrap()
            .model
            .as_deref(),
        Some("cli/explicit")
    );
    assert_eq!(
        store.get_session_model_overrides(id).await.unwrap(),
        Some(SessionModelOverrides::default())
    );
}

fn explicit_request() -> ChatRequest {
    ChatRequest {
        purpose: opencoder_llm::RequestPurpose::Conversation,
        model: "cli/explicit".into(),
        messages: Vec::new(),
        tools: Vec::new(),
        tool_choice: None,
        temperature: None,
        max_tokens: None,
        reasoning_effort: None,
        cache_salt: None,
    }
}

#[tokio::test]
async fn explicit_model_survives_resume_and_fork_without_losing_generation_overlays() {
    let dir = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::scoped_config_home(dir.path().join("config-home"));
    write_config(&dir, &disk_config());
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    create_target(&store, "resume-target").await;
    create_target(&store, "fork-source").await;
    let mut config = disk_config();
    config.model = "old/shared".into();
    let mut client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let initial = SessionState::new(
        "current",
        resolve_agent("act").unwrap(),
        config.clone(),
        client.clone(),
        dir.path().to_path_buf(),
    );
    let mut session_id = initial.id.clone();
    let mut chat = ChatView {
        agent: "act".into(),
        model: "old/shared".into(),
        shared_model: "old/shared".into(),
        ..Default::default()
    };
    let (mut cmd_tx, _old_commands) = mpsc::channel(8);
    let (ui_tx, mut evt_rx) = mpsc::channel(16);
    let mut cancel = CancellationToken::new();
    let first_cancel = cancel.clone();
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
    let explicit = Some("cli/explicit".to_string());

    macro_rules! switch {
        ($pick:expr, $model:expr) => {
            switch_session(
                $pick,
                &TuiOpts::default(),
                $model,
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
        };
    }

    let state = switch!(
        crate::task::TaskPick::Resume("resume-target".into()),
        &explicit
    )
    .await
    .unwrap();
    assert!(first_cancel.is_cancelled());
    assert_eq!(state.config.model, "cli/explicit");
    assert_eq!(state.config.modes.shared.model, None);
    assert_eq!(state.config.modes.shared.max_tokens, Some(1_100));
    let act = state.config.modes.act.as_ref().unwrap();
    assert_eq!(act.model, None);
    assert_eq!(act.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(act.max_tokens, Some(2_200));
    assert_eq!(
        state.model_overrides,
        Some(SessionModelOverrides::default())
    );
    assert_eq!(state.shared_model, "cli/explicit");
    assert_eq!(state.model, "cli/explicit");
    assert_eq!(state.context_limit, 22_000);
    assert_eq!(state.compaction_threshold, 73_000);
    assert_eq!(
        state.client.request_body(&explicit_request()).unwrap()["model"],
        "explicit"
    );
    assert!(!Arc::ptr_eq(&state.client, &client));
    config = state.config;
    client = state.client;
    assert_eq!(chat.model, "cli/explicit");
    assert_eq!(chat.shared_model, "cli/explicit");
    assert_explicit_row(&store, "resume-target").await;

    let state = switch!(crate::task::TaskPick::Fork("fork-source".into()), &explicit)
        .await
        .unwrap();
    assert_ne!(session_id, "fork-source");
    assert_eq!(state.config.modes.shared.context_limit, Some(11_000));
    assert_eq!(
        state.config.modes.act.as_ref().unwrap().max_tokens,
        Some(2_200)
    );
    assert_eq!(state.context_limit, 22_000);
    assert_eq!(
        state.model_overrides,
        Some(SessionModelOverrides::default())
    );
    assert_eq!(state.model, "cli/explicit");
    assert_explicit_row(&store, &session_id).await;
    assert_original_row(&store, "fork-source").await;
    config = state.config;
    client = state.client;
    assert_eq!(config.model, "cli/explicit");
    assert_eq!(
        client.request_body(&explicit_request()).unwrap()["model"],
        "explicit"
    );
    cmd_tx.send(UiCmd::Quit).await.unwrap();
}

#[tokio::test]
async fn failed_explicit_resume_preserves_current_runtime_and_target_models() {
    let dir = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::scoped_config_home(dir.path().join("config-home"));
    let inner = Arc::new(TursoStore::open_memory().await.unwrap());
    let store: Arc<dyn Store> = inner.clone();
    create_target(&store, "resume-target").await;
    let mut broken = disk_config();
    broken.provider.api_key = None;
    broken.providers.get_mut("cli").unwrap().api_key = None;
    write_config(&dir, &broken);
    let config = disk_config();
    let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let old_client = client.clone();
    let initial = SessionState::new(
        "current",
        resolve_agent("act").unwrap(),
        config.clone(),
        client.clone(),
        dir.path().to_path_buf(),
    );
    let mut session_id = initial.id.clone();
    let mut chat = ChatView {
        agent: "act".into(),
        model: "old/act".into(),
        shared_model: "old/shared".into(),
        model_overrides: Some(stored_overrides()),
        ..Default::default()
    };
    let (mut cmd_tx, mut old_commands) = mpsc::channel(8);
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
    let mut history = vec!["old history".into()];
    let (mut scroll, mut follow, mut queue_scroll, mut sys) = (4, false, 2, 99);
    let (mut queue, mut skill, mut body) = (Vec::new(), None, None);
    let (mut input, mut cursor, mut hist_idx) = ("old draft".to_string(), 4, Some(0));
    let explicit = Some("cli/explicit".to_string());

    macro_rules! switch {
        ($pick:expr, $model:expr) => {
            switch_session(
                $pick,
                &TuiOpts::default(),
                $model,
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
        };
    }

    let error = switch!(
        crate::task::TaskPick::Resume("resume-target".into()),
        &explicit
    )
    .await
    .err()
    .expect("client construction must fail");
    assert!(error.to_string().contains("missing API key"), "{error:#}");
    assert_original_row(&store, "resume-target").await;

    write_config(&dir, &disk_config());
    let conn = inner.conn().await.unwrap();
    conn.execute(
        "CREATE TRIGGER fail_model_metadata BEFORE UPDATE OF model_overrides_json ON sessions \
         WHEN OLD.id = 'resume-target' BEGIN SELECT RAISE(ABORT, 'injected model metadata failure'); END",
        (),
    )
    .await
    .unwrap();
    drop(conn);
    let error = switch!(
        crate::task::TaskPick::Resume("resume-target".into()),
        &explicit
    )
    .await
    .err()
    .expect("metadata persistence must fail");
    assert!(format!("{error:#}").contains("injected model metadata failure"));
    assert_original_row(&store, "resume-target").await;

    assert_eq!(session_id, "current");
    assert!(running);
    assert!(!old_cancel.is_cancelled());
    assert!(Arc::ptr_eq(&client, &old_client));
    assert_eq!(config.model, "stored/config");
    assert_eq!(chat.model, "old/act");
    assert_eq!(chat.shared_model, "old/shared");
    assert_eq!(chat.model_overrides, Some(stored_overrides()));
    assert_eq!(history, vec!["old history"]);
    assert_eq!(input, "old draft");
    assert!(matches!(
        old_commands.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

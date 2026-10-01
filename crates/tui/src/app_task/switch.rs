use super::*;

#[tokio::test]
async fn agent_self_detaches_remote_and_creates_an_empty_local_task_then_resumes_only_remote_context(
) {
    let dir = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::scoped_config_home(dir.path().join("config-home"));
    let store: Arc<dyn Store> =
        Arc::new(opencoder_store::LibsqlStore::open_memory().await.unwrap());
    let client: Arc<dyn ChatStream> = Arc::new(MockChatClient::new());
    let mut config = Config::default();
    config.opencoder_server.enabled = true;
    config.opencoder_server.url = "http://127.0.0.1:1".into();
    std::fs::write(
        dir.path().join("opencoder.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let binding = opencoder_core::harness::RemoteSession {
        server_url: config.opencoder_server.url.clone(),
        capability: opencoder_core::harness::ServerCapability {
            id: "ops".into(),
            kind: opencoder_core::fleet::ExecutionKind::Operator,
            target: "codex".into(),
            summary: "Ops".into(),
        },
        created: false,
        initial_input: None,
    };
    let session = crate::remote::create(
        binding,
        config.clone(),
        client.clone(),
        store.clone(),
        dir.path(),
    )
    .await
    .unwrap();
    let remote_id = session.id.clone();
    let mut session_id = remote_id.clone();
    let mut chat = ChatView {
        agent: "operator:ops".into(),
        remote: true,
        ..Default::default()
    };
    chat.push_marker(Line::from("remote-only content"));
    chat.tokens_total = 999;
    let (mut cmd_tx, mut old_commands) = mpsc::channel(8);
    let (_ui, mut evt_rx) = mpsc::channel(16);
    let mut cancel = CancellationToken::new();
    let old_cancel = cancel.clone();
    let mut turn_cancel = Arc::new(Mutex::new(CancellationToken::new()));
    let mut children = ChildRuntimeHandles::from_session(&session);
    let mut skill_handle = session.skill_prompt.clone();
    let mut hub = session.question_hub.clone();
    let mut sidecar = crate::sidecar_ui::spawn_actor(&session, _ui, Some(store.clone()));
    let mut label = String::new();
    let mut snapshots = std::collections::HashMap::new();
    let mut running = true;
    let mut history = vec!["remote history".into()];
    let mut scroll = 4;
    let mut follow = false;
    let mut queue_scroll = 1;
    let mut sys = 123;
    let mut queue = vec![];
    let mut skill = None;
    let mut body = None;
    let mut input = "remote draft".into();
    let mut cursor = 12;
    let mut hist_idx = Some(0);
    switch_session(
        crate::task::TaskPick::New,
        &TuiOpts::default(),
        &mut cmd_tx,
        &mut evt_rx,
        dir.path(),
        &config,
        &client,
        &store,
        &mut label,
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
    assert!(
        !old_cancel.is_cancelled(),
        "detaching must not interrupt Server"
    );
    assert!(matches!(old_commands.recv().await, Some(UiCmd::Quit)));
    assert_ne!(session_id, remote_id);
    assert!(!chat.remote);
    assert!(chat.blocks.is_empty());
    assert_eq!(chat.tokens_total, 0);
    assert!(history.is_empty());
    assert!(input.is_empty());
    assert!(!running);
    assert!(store.get_session(&session_id).await.unwrap().is_some());
    assert!(store
        .harness_runtime(&session_id)
        .await
        .unwrap()
        .unwrap()
        .remote
        .is_none());
    chat.push_marker(Line::from("self-only content"));
    history.push("local history".into());
    switch_session(
        crate::task::TaskPick::Resume(remote_id.clone()),
        &TuiOpts::default(),
        &mut cmd_tx,
        &mut evt_rx,
        dir.path(),
        &config,
        &client,
        &store,
        &mut label,
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
    assert_eq!(session_id, remote_id);
    assert!(chat.remote);
    assert_eq!(chat.agent, "operator:ops");
    assert_eq!(history, vec!["remote history"]);
    assert!(
        chat.blocks.is_empty(),
        "Server transcript is loaded by remote worker, no local blocks"
    );
    cmd_tx.send(UiCmd::Quit).await.unwrap();
}

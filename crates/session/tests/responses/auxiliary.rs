use super::wire::*;
use opencoder_core::{resolve_agent, ContentBlock};
use opencoder_session::{run, SessionState};
use opencoder_store::{LibsqlStore, Store};
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn child_tools_and_parent_continuation_use_responses_and_keep_separate_state() {
    let dir = tempfile::tempdir().unwrap();
    let _home = opencoder_core::scoped_config_home(dir.path().to_path_buf());
    let server = serve(vec![
        Reply::events(vec![completed(vec![
            reasoning(),
            call(
                "task1",
                "task",
                json!({"prompt":"inspect files", "subagent_type":"explore"}),
            ),
        ])]),
        Reply::events(vec![completed(vec![
            reasoning(),
            call("ls1", "bash", json!({"command":"echo child-result"})),
        ])]),
        Reply::events(vec![completed(vec![reasoning(), answer("child finished")])]),
        Reply::events(vec![completed(vec![answer("parent finished")])]),
    ])
    .await;
    let config = config(&server.url);
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut session = SessionState::new(
        "parent",
        resolve_agent("act").unwrap(),
        config.clone(),
        Arc::new(client(&config)),
        dir.path().to_path_buf(),
    )
    .with_store(store.clone());
    run(&mut session, "Inspect with a child".into(), |_| {})
        .await
        .unwrap();
    let tasks = store.list_subagent_tasks("parent").await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].ok, Some(true));
    let messages = store
        .load_messages(&tasks[0].child_session_id)
        .await
        .unwrap();
    assert_eq!(
        messages
            .iter()
            .filter(|m| m.provider_state.is_some())
            .count(),
        2
    );
    assert!(messages.iter().flat_map(|m| &m.blocks).any(
        |b| matches!(b, ContentBlock::ToolResult{content,..} if content.contains("child-result"))
    ));
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests
        .iter()
        .all(|(headers, _)| headers.starts_with("POST /responses ")));
    let child = requests[2].1["input"].as_array().unwrap();
    assert!(child
        .iter()
        .any(|v| v["type"] == "function_call_output" && v["call_id"] == "ls1"));
    let parent = requests[3].1["input"].as_array().unwrap();
    assert!(parent
        .iter()
        .any(|v| v["type"] == "function_call_output" && v["call_id"] == "task1"));
    assert!(!parent.iter().any(|v| v["call_id"] == "ls1"));
}

#[tokio::test]
async fn title_and_verify_route_to_responses_small_model_with_usable_output_budget() {
    let dir = tempfile::tempdir().unwrap();
    let _home = opencoder_core::scoped_config_home(dir.path().to_path_buf());
    let server = serve(vec![
        Reply::json(response(vec![answer("Fixed task")])),
        Reply::json(response(vec![answer("yes")])),
    ])
    .await;
    let mut config = config("http://127.0.0.1:1");
    config.provider.protocol = "chat_completions".into();
    config.small_model = Some("small/gpt-6".into());
    config.providers.insert(
        "small".into(),
        opencoder_core::ProviderConfig {
            protocol: "responses".into(),
            base_url: server.url.clone(),
            ..Default::default()
        },
    );
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&opencoder_store::SessionMeta {
            id: "aux".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut session = SessionState::new(
        "aux",
        resolve_agent("act").unwrap(),
        config.clone(),
        Arc::new(client(&config)),
        dir.path().to_path_buf(),
    )
    .with_store(store.clone());
    session
        .messages
        .push(opencoder_core::Message::user("u", "Fix task"));
    opencoder_session::generate_title(&session).await;
    assert_eq!(
        store
            .get_session("aux")
            .await
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("Fixed task")
    );
    let verdict = opencoder_session::autopilot::verify(
        &session,
        &opencoder_session::autopilot::state::ApState::new("Fix task".into()),
        1,
    )
    .await
    .unwrap();
    assert!(matches!(
        verdict,
        opencoder_session::autopilot::VerifyVerdict::Complete
    ));
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for (_, body) in requests.iter() {
        assert_eq!(body["model"], "gpt-6");
        assert_eq!(body["reasoning"]["effort"], "low");
        assert_eq!(body["max_output_tokens"], 4096);
        assert!(body.get("temperature").is_none());
    }
}

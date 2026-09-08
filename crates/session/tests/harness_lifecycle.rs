#![cfg(unix)]
#[path = "harness/fixtures.rs"]
mod fixtures;
use opencoder_core::{harness::Harness, Config, Message};
use opencoder_session::{
    control_cmd::{apply, ControlCmd},
    run,
};

#[tokio::test]
async fn agent_default_applies_to_new_sessions_and_legacy_history_stays_native() {
    let root = tempfile::tempdir().unwrap();
    let resources = tempfile::tempdir().unwrap();
    fixtures::card(resources.path());
    let (mut session, store) = fixtures::session(root.path()).await;
    session.harness = Default::default();
    session
        .record_checked(Message::user("old", "existing native history"))
        .await
        .unwrap();
    let mut config = Config::default();
    config.agent.default = "custom".into();
    config.agent.agents_dir = Some(resources.path().into());
    let legacy = opencoder_session::resume(
        store.clone(),
        &session.id,
        config.clone(),
        session.client.clone(),
        root.path().into(),
    )
    .await
    .unwrap();
    assert_eq!(legacy.harness.harness, Harness::Opencoder);
    let fresh =
        opencoder_core::agent::scope::with_root_sync(config.agent.agents_dir.clone(), || {
            opencoder_session::SessionState::new(
                "fresh",
                opencoder_core::resolve_agent("custom").unwrap(),
                config,
                session.client.clone(),
                root.path().into(),
            )
        });
    assert_eq!(fresh.harness.harness, Harness::Codex);
    opencoder_session::harness::initialize(
        store.as_ref(),
        &session.id,
        "act",
        Some(Harness::Opencoder),
        Default::default(),
    )
    .await
    .unwrap();
    let error = opencoder_session::harness::initialize(
        store.as_ref(),
        &session.id,
        "act",
        Some(Harness::Codex),
        Default::default(),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("fixed"));
}

#[tokio::test]
async fn unknown_submission_is_not_repeated_or_forked() {
    let root = tempfile::tempdir().unwrap();
    let (mut session, store) = fixtures::session(root.path()).await;
    run(&mut session, "first".into(), |_| {}).await.unwrap();
    session.harness.in_flight = true;
    session.harness.thread_id = None;
    opencoder_session::harness::save(&session).await.unwrap();
    let mut resumed = opencoder_session::resume(
        store.clone(),
        &session.id,
        Config::default(),
        session.client.clone(),
        root.path().into(),
    )
    .await
    .unwrap();
    let error = run(&mut resumed, "new".into(), |_| {}).await.unwrap_err();
    assert!(error.to_string().contains("unknown without a thread ID"));
    assert_eq!(
        std::fs::read_to_string(root.path().join("capture.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert!(opencoder_session::fork::fork_session_with_id(
        store.as_ref(),
        &session.id,
        "fork-unknown"
    )
    .await
    .is_err());
    assert!(store.get_session("fork-unknown").await.unwrap().is_none());
}

#[tokio::test]
async fn clear_context_starts_a_new_codex_thread_with_current_agent() {
    let root = tempfile::tempdir().unwrap();
    let (mut session, store) = fixtures::session(root.path()).await;
    session.agent = opencoder_core::resolve_agent("plan").unwrap();
    run(&mut session, "plan".into(), |_| {}).await.unwrap();
    assert!(apply(
        &mut session,
        &ControlCmd::SwitchAgent("act".into()),
        &mut |_| {}
    )
    .await
    .is_err());
    apply(&mut session, &ControlCmd::ClearContext, &mut |_| {})
        .await
        .unwrap();
    let mut resumed = opencoder_session::resume(
        store,
        &session.id,
        Config::default(),
        session.client.clone(),
        root.path().into(),
    )
    .await
    .unwrap();
    assert_eq!(resumed.agent.name, "act");
    assert!(resumed.harness.thread_id.is_none());
    run(&mut resumed, "execute".into(), |_| {}).await.unwrap();
    let records: Vec<serde_json::Value> =
        std::fs::read_to_string(root.path().join("capture.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
    assert_eq!(records[1]["args"][1], "--json");
    assert!(records[1]["prompt"]
        .as_str()
        .unwrap()
        .contains("# Agent instructions"));
}

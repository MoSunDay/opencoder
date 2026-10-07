#![cfg(unix)]
#[path = "harness/fixtures.rs"]
mod fixtures;

use opencoder_session::{run, SessionEvent};

#[tokio::test]
async fn codex_completion_does_not_start_an_act_memory_session() {
    let root = tempfile::tempdir().unwrap();
    let resources = tempfile::tempdir().unwrap();
    fixtures::card(resources.path());
    let (mut session, store) = fixtures::session(root.path()).await;
    session.config.agent.agents_dir = Some(resources.path().into());
    session.agent =
        opencoder_core::agent::scope::with_root_sync(Some(resources.path().into()), || {
            opencoder_core::resolve_agent("custom").unwrap()
        });
    session.config.local_memory = true;
    let mut events = Vec::new();
    run(
        &mut session,
        "complete external harness task".into(),
        |event| events.push(event),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(root.path().join("capture.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert!(!events.iter().any(
        |event| matches!(event, SessionEvent::SubagentStart { kind, .. } if kind == "memory")
    ));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, SessionEvent::Done))
            .count(),
        1
    );
    assert!(store
        .load_messages(&session.id)
        .await
        .unwrap()
        .iter()
        .any(|message| message.role == opencoder_core::Role::Assistant));
    assert!(
        session.config.local_memory,
        "the user's native memory setting is retained"
    );
}

#[tokio::test]
async fn codex_failure_stays_failed_with_native_memory_enabled() {
    let root = tempfile::tempdir().unwrap();
    let (mut session, _) = fixtures::session(root.path()).await;
    session.config.local_memory = true;
    session
        .harness
        .envs
        .insert("FAIL_MODE".into(), "missing_end".into());
    let mut events = Vec::new();
    assert!(
        run(&mut session, "fail external task".into(), |event| events
            .push(event))
        .await
        .is_err()
    );
    assert!(!events.iter().any(|event| matches!(
        event,
        SessionEvent::Done | SessionEvent::SubagentStart { .. }
    )));
}

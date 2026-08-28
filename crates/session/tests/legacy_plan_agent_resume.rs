//! Regression: a store session row persisted with the removed `plan` agent
//! (pre-refactor plan mode) resumes through the session layer as the `act`
//! agent — the pinned default. No panic, no dead agent reference, and the
//! resumed session runs a normal turn.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{LlmEvent, MockChatClient, Usage};
use opencoder_session::{resume, run, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

#[tokio::test]
async fn legacy_plan_row_resumes_as_act_agent() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "legacy-plan".into(),
            // The removed agent name, exactly as old rows persisted it.
            agent: Some("plan".into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .append_messages(
            "legacy-plan",
            &[opencoder_core::Message::user("u1", "old plan-mode work")],
        )
        .await
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let mock = Arc::new(MockChatClient::new().push_script(vec![LlmEvent::Completed {
        text: "resumed fine".into(),
        tool_calls: vec![],
        usage: Some(Usage::default()),
    }]));
    let resumed: SessionState = resume(
        store.clone(),
        "legacy-plan",
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .await
    .unwrap();

    // The removed agent resolves to the default execution agent.
    assert_eq!(
        resumed.agent.name, "act",
        "legacy `plan` rows must resume as the act agent"
    );
    assert_eq!(
        resumed.agent.kind,
        opencoder_core::AgentKind::Act,
        "resolved agent carries the act kind (no sandbox-style write guard)"
    );
    assert!(
        resolve_agent("plan").is_none(),
        "precondition: `plan` no longer resolves as a builtin agent"
    );

    // The resumed session is fully functional.
    let mut runnable = resumed;
    run(&mut runnable, "continue".into(), |_| {}).await.unwrap();
}

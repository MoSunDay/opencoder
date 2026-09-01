//! Regression: a store session row persisted with the interlude `sandbox`
//! agent resumes through the session layer as the restored `plan` agent. No
//! panic, no dead agent reference, and the resumed session runs a normal
//! turn with the plan write guard active.

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
async fn legacy_sandbox_row_resumes_as_plan_agent() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "legacy-sandbox".into(),
            // The interlude agent name, exactly as those rows persisted it.
            agent: Some("sandbox".into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .append_messages(
            "legacy-sandbox",
            &[opencoder_core::Message::user("u1", "old sandbox-mode work")],
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
        "legacy-sandbox",
        config(),
        mock,
        dir.path().to_path_buf(),
    )
    .await
    .unwrap();

    // The interlude name resolves to the restored plan agent.
    assert_eq!(
        resumed.agent.name, "plan",
        "legacy `sandbox` rows must resume as the plan agent"
    );
    assert_eq!(
        resumed.agent.kind,
        opencoder_core::AgentKind::Plan,
        "resolved agent carries the plan kind (write guard stays active)"
    );
    assert!(
        resolve_agent("sandbox").is_none(),
        "precondition: `sandbox` no longer resolves as a builtin agent"
    );

    // The resumed session is fully functional.
    let mut runnable = resumed;
    run(&mut runnable, "continue".into(), |_| {}).await.unwrap();
}

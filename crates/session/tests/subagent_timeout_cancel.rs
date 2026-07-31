//! Integration test: a subagent that exceeds `task_timeout` must end up
//! Cancelled in the DB — not Completed (the old bug) and never stuck Running.
//!
//! Root cause: `TaskSignal::Timeout` was the only signal that did not fire a
//! cancel token to the child. If the child finished during the grace drain
//! window, its own cleanup marked it Completed, silently swallowing the
//! timeout. The fix fires the child's hard-cancel token on timeout and
//! overrides the DB status to Cancelled in the `Ok(o)` drain path.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, Store, SubagentStatus};

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config() -> Config {
    Config {
        model: "m/g".into(),
        // 1s task deadline: the child's `sleep 2` bash (backgrounded at 1s
        // under cfg(test)) plus its pending second turn keep the subagent
        // alive past this, so Phase 1's `deadline` arm wins.
        task_timeout_secs: Some(1),
        // Generous drain so the child finishes its cleanup *inside* the grace
        // window — exercises the `Ok(o)` override-to-Cancelled path rather
        // than the force-cancel (Err) fallback.
        subagent_drain_secs: Some(10),
        ..Config::default()
    }
}

fn task_turn(prompt: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: "delegating".into(),
        tool_calls: vec![CompletedToolCall {
            id: "task-1".into(),
            name: "task".into(),
            input: serde_json::json!({"prompt": prompt, "subagent_type": "explore"}),
        }],
        usage: Some(Usage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
            ..Default::default()
        }),
    }
}

fn bash_call(cmd: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: String::new(),
        tool_calls: vec![CompletedToolCall {
            id: "bash-1".into(),
            name: "bash".into(),
            input: serde_json::json!({"command": cmd}),
        }],
        usage: Some(Usage {
            input_tokens: 5,
            output_tokens: 5,
            total_tokens: 10,
            ..Default::default()
        }),
    }
}

fn text_done(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: Some(Usage {
            input_tokens: 5,
            output_tokens: 5,
            total_tokens: 10,
            ..Default::default()
        }),
    }
}

#[tokio::test]
async fn timeout_marks_subagent_cancelled() {
    let store = mem_store().await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![task_turn("explore something")])
            .push_script(vec![bash_call("sleep 2")])
            .with_default(vec![text_done("done")]),
    ) as Arc<dyn ChatStream>;

    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        "timeout-cancel-test",
        agent,
        config(),
        mock,
        std::env::temp_dir(),
    )
    .with_store(store.clone());
    let session_id = session.id.clone();

    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();

    // Bound the run so a regression (e.g. a wedged child) fails fast instead
    // of stalling the suite. With the fix the run completes in ~1-2s; the
    // 30s ceiling comfortably absorbs scheduler jitter.
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        run(&mut session, "go".into(), move |ev| {
            events_clone.lock().unwrap().push(ev);
        }),
    )
    .await;
    assert!(
        result.is_ok(),
        "run did not complete within 30s; subagent timeout drain is broken"
    );

    // The subagent task must be Cancelled, not Completed or Running.
    let tasks = store.list_subagent_tasks(&session_id).await.unwrap();
    assert_eq!(tasks.len(), 1, "expected exactly one subagent task");
    assert!(
        matches!(tasks[0].status, SubagentStatus::Cancelled),
        "task must be Cancelled after timeout, got {:?}",
        tasks[0].status
    );
}

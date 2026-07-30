//! Integration test: a parent steer cancels a running child subagent via
//! `fire_child_cancels`, causing `run_subagent` to return `err("cancelled")`
//! and the parent to absorb the steer at the next turn boundary.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{fire_child_cancels, run, SessionEvent, SessionState};
use tokio_util::sync::CancellationToken;

fn config() -> Config {
    Config {
        model: "m/g".into(),
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
async fn parent_steer_cancels_running_child() {
    // Parent dispatches a subagent that runs bash("sleep 30"). After the child
    // starts, we fire_child_cancels. The child's cancel token fires, the bash
    // tool's select! drops the sleep future (kill_on_drop), the child's run_loop
    // breaks at the top-of-loop cancel check, and run_subagent returns
    // err("cancelled"). The parent then continues its own turn.
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![task_turn("explore something")])
            .push_script(vec![bash_call("sleep 30")])
            .push_script(vec![text_done("recovered")]),
    ) as Arc<dyn ChatStream>;

    let agent = resolve_agent("act").unwrap();
    let mut session =
        SessionState::new("child-cancel", agent, config(), mock, std::env::temp_dir());

    let child_cancels = session.child_cancels.clone();
    let parent_cancel = CancellationToken::new();
    session = session.with_cancel(parent_cancel.clone());

    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();

    let mut session = session;
    let handle = tokio::spawn(async move {
        run(&mut session, "go".into(), move |ev| {
            events_clone.lock().unwrap().push(ev);
        })
        .await
    });

    // Wait for SubagentStart so we know the child is registered.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, SessionEvent::SubagentStart { .. }))
        {
            break;
        }
        if Instant::now() > deadline {
            panic!("subagent never started within 5s");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Let the child reach the bash command before cancelling.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let any_cancelled = fire_child_cancels(&child_cancels);
    assert!(any_cancelled, "expected at least one child registered");

    // Must finish well under the 30s sleep.
    let result = tokio::time::timeout(Duration::from_secs(10), handle).await;
    assert!(
        result.is_ok(),
        "run did not complete within 10s; child cancellation failed"
    );

    let evs = events.lock().unwrap();

    let cancelled_end = evs
        .iter()
        .any(|e| matches!(e, SessionEvent::SubagentEnd { cancelled: true, .. }));
    assert!(
        cancelled_end,
        "expected SubagentEnd with cancelled=true"
    );

    let task_error = evs.iter().any(|e| {
        matches!(
            e,
            SessionEvent::ToolEnd {
                name,
                is_error: true,
                ..
            } if name == "task"
        )
    });
    assert!(
        task_error,
        "expected ToolEnd(task, is_error=true) after child cancelled"
    );

    assert!(
        !parent_cancel.is_cancelled(),
        "parent's own cancel token must remain intact"
    );
}

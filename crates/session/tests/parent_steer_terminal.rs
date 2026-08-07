//! G2 regression: a parent steer (TUI `>` / web POST /prompt with running
//! children) must cancel running subagents and mark their tasks TERMINAL
//! (Failed), not Cancelled.
//!
//! Before the fix, `run_subagent` detected child cancellation but could not
//! distinguish steer from hard-abort, so it always called
//! `cancel_subagent_task` — leaving the task replayable (Cancelled) even
//! though the parent was still alive. On the next turn the task would be
//! silently replayed, duplicating work.
//!
//! After the fix, `run_subagent` checks the parent's own `cancel` token: if
//! the parent is NOT cancelled (steer path), it calls
//! `complete_subagent_task(.., false)` → terminal Failed, records a real
//! tool_result, and the task is never replayed.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{fire_child_cancels, run, SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, Store, SubagentStatus};
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

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

/// Wait until the events vector contains a SubagentStart, or panic after 5s.
async fn wait_for_subagent_start(events: &Arc<Mutex<Vec<SessionEvent>>>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, SessionEvent::SubagentStart { .. }))
        {
            return;
        }
        if Instant::now() > deadline {
            panic!("subagent never started within 5s");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A parent steer cancels only the child token. The child task must become
/// terminal `Failed` (not `Cancelled`), the parent's own cancel must stay
/// intact, and the parent must continue its run cleanly.
#[tokio::test]
async fn parent_steer_makes_subagent_task_terminal_failed() {
    let store = mem_store().await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![task_turn("explore the codebase")])
            .push_script(vec![bash_call("sleep 30")])
            .push_script(vec![text_done("recovered after steer")]),
    ) as Arc<dyn ChatStream>;

    let agent = resolve_agent("act").unwrap();
    let cancel = CancellationToken::new();
    let session_id = "parent-steer-g2-1".to_string();
    let session = SessionState::new(
        session_id.clone(),
        agent,
        config(),
        mock,
        std::env::temp_dir(),
    )
    .with_store(store.clone())
    .with_cancel(cancel.clone());

    let child_cancels = session.child_cancels.clone();
    let child_turn_cancels = session.child_turn_cancels.clone();
    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();

    let mut session = session;
    let handle = tokio::spawn(async move {
        run(&mut session, "go".into(), move |ev| {
            events_clone.lock().unwrap().push(ev);
        })
        .await
    });

    // Wait for the subagent to start, then let the child reach bash, then steer.
    wait_for_subagent_start(&events).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Parent steer: cancel only the child token, NOT the parent's cancel.
    let fired = fire_child_cancels(&child_cancels);
    assert!(
        fired,
        "fire_child_cancels should have found the running child"
    );

    // Must finish well under the 30s sleep.
    let result = tokio::time::timeout(Duration::from_secs(15), handle).await;
    assert!(
        result.is_ok(),
        "run did not complete within 15s; parent steer during subagent is broken"
    );

    // The parent's own cancel must NOT have been fired (no hard abort).
    assert!(
        !cancel.is_cancelled(),
        "parent cancel must remain intact after a steer (only the child token fired)"
    );

    // Registry entries must be pruned by the cleanup path.
    assert!(
        child_cancels.lock().unwrap().is_empty(),
        "child_cancels registry must be empty after cleanup"
    );
    assert!(
        child_turn_cancels.lock().unwrap().is_empty(),
        "child_turn_cancels registry must be empty after cleanup"
    );

    {
        let evs = events.lock().unwrap();

        // The steer path emits SubagentEnd with cancelled=true and a
        // steer-specific summary.
        let steer_end = evs.iter().any(|e| {
            matches!(
                e,
                SessionEvent::SubagentEnd {
                    cancelled: true,
                    summary,
                    ..
                } if summary == "cancelled: redirected by parent steer"
            )
        });
        assert!(
            steer_end,
            "expected SubagentEnd with cancelled=true and steer summary"
        );
    }

    // Root-cause fix: the task must be TERMINAL (Failed), not Cancelled.
    let tasks = store.list_subagent_tasks(&session_id).await.unwrap();
    assert_eq!(tasks.len(), 1, "expected exactly one subagent task");
    assert!(
        matches!(tasks[0].status, SubagentStatus::Failed),
        "task must be Failed (terminal) after parent steer, got {:?}",
        tasks[0].status
    );
}

/// After a steer-cancelled subagent, the parent continues cleanly and the
/// terminal-Failed task is never replayed on a subsequent turn.
#[tokio::test]
async fn continue_after_parent_steer_does_not_replay() {
    let store = mem_store().await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![task_turn("explore something")])
            .push_script(vec![bash_call("sleep 30")])
            .push_script(vec![text_done("ok after steer")])
            // Scripts for the second run — must NOT trigger a subagent.
            .push_script(vec![text_done("second turn done")]),
    ) as Arc<dyn ChatStream>;

    let agent = resolve_agent("act").unwrap();
    let cancel = CancellationToken::new();
    let session_id = "parent-steer-g2-2".to_string();
    let mut session = SessionState::new(
        session_id.clone(),
        agent,
        config(),
        mock,
        std::env::temp_dir(),
    )
    .with_store(store.clone())
    .with_cancel(cancel.clone());

    let child_cancels = session.child_cancels.clone();
    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();

    let handle = tokio::spawn(async move {
        let res = run(&mut session, "go".into(), move |ev| {
            events_clone.lock().unwrap().push(ev);
        })
        .await;
        (res, session)
    });

    // Wait for subagent start, then steer.
    wait_for_subagent_start(&events).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    fire_child_cancels(&child_cancels);

    let (_res, mut session) = tokio::time::timeout(Duration::from_secs(15), handle)
        .await
        .expect("first run did not complete within 15s")
        .expect("first run panicked");

    // The first task is terminal Failed.
    let tasks = store.list_subagent_tasks(&session_id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(matches!(tasks[0].status, SubagentStatus::Failed));

    // Second turn: the task must NOT be replayed (no new SubagentStart).
    let events2: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events2_clone = events2.clone();
    let handle2 = tokio::spawn(async move {
        run(&mut session, "next".into(), move |ev| {
            events2_clone.lock().unwrap().push(ev);
        })
        .await
    });

    let result = tokio::time::timeout(Duration::from_secs(10), handle2).await;
    assert!(result.is_ok(), "second run did not complete within 10s");

    {
        let evs2 = events2.lock().unwrap();
        let replayed = evs2
            .iter()
            .any(|e| matches!(e, SessionEvent::SubagentStart { .. }));
        assert!(
            !replayed,
            "terminal-Failed task must not be replayed on subsequent turn"
        );
    }

    // Still exactly one task in the DB (no duplicate).
    let tasks2 = store.list_subagent_tasks(&session_id).await.unwrap();
    assert_eq!(tasks2.len(), 1, "no new subagent task should be created");
}

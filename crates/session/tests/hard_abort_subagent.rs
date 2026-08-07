//! Integration tests for the hard-abort path when a subagent is mid-flight.
//!
//! When the parent's cancel token fires (double-Esc) while a child subagent is
//! running a long tool, the two-phase select in `execute_call_with_timeout`
//! lets the parent proceed without hanging on the child. The child's cleanup
//! path (`run_subagent`) prunes the `child_cancels`/`child_turn_cancels`
//! registries, marks the task `Cancelled`, and emits `SubagentEnd { cancelled:
//! true }`. A subsequent turn must not hang on the stale child, and the
//! abandoned task must never be left `Running` (which would later cause a
//! provider HTTP 400 from a dangling, never-answered `task` tool_use).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, Store, SubagentStatus};
use tokio_util::sync::CancellationToken;

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config() -> Config {
    Config {
        model: "m/g".into(),
        // Short grace window so a wedged subagent is force-cancelled quickly
        // instead of stalling the test for the full default 15s.
        subagent_drain_secs: Some(2),
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

/// Block until the runner has emitted a `SubagentStart` (the child registered
/// its cancel tokens), bounding the wait so a broken dispatch fails fast.
async fn wait_for_subagent_start(events: &Arc<Mutex<Vec<SessionEvent>>>) {
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
}

#[tokio::test]
async fn hard_abort_during_subagent_marks_task_cancelled() {
    let store = mem_store().await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![task_turn("explore something")])
            .push_script(vec![bash_call("sleep 30")])
            .push_script(vec![text_done("recovered")]),
    ) as Arc<dyn ChatStream>;

    let agent = resolve_agent("act").unwrap();
    let cancel = CancellationToken::new();
    let mut session = SessionState::new(
        "hard-abort-sub",
        agent,
        config(),
        mock,
        std::env::temp_dir(),
    )
    .with_cancel(cancel.clone())
    .with_store(store.clone());
    let child_cancels = session.child_cancels.clone();
    let child_turn_cancels = session.child_turn_cancels.clone();
    let session_id = session.id.clone();

    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();
    let handle = tokio::spawn(async move {
        run(&mut session, "go".into(), move |ev| {
            events_clone.lock().unwrap().push(ev);
        })
        .await
    });

    // Wait for the subagent to start, let the child reach bash, then cancel.
    wait_for_subagent_start(&events).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    cancel.cancel();

    // Must finish well under the 30s sleep.
    let result = tokio::time::timeout(Duration::from_secs(15), handle).await;
    assert!(
        result.is_ok(),
        "run did not complete within 15s; hard-abort during subagent is broken"
    );

    {
        let evs = events.lock().unwrap();

        // Key fix: the cleanup path ran, emitting SubagentEnd with cancelled=true.
        let cancelled_end = evs.iter().any(|e| {
            matches!(
                e,
                SessionEvent::SubagentEnd {
                    cancelled: true,
                    ..
                }
            )
        });
        assert!(
            cancelled_end,
            "expected SubagentEnd with cancelled=true after hard abort"
        );

        let saw_interrupted = evs
            .iter()
            .any(|e| matches!(e, SessionEvent::Status(msg) if msg == "interrupted"));
        assert!(saw_interrupted, "expected Status(interrupted) after cancel");
    }

    // Registry entries must be pruned by the cleanup path.
    assert!(
        child_cancels.lock().unwrap().is_empty(),
        "child_cancels registry must be empty after cleanup"
    );
    assert!(
        child_turn_cancels.lock().unwrap().is_empty(),
        "child_turn_cancels registry must be empty after cleanup"
    );

    // Root-cause fix: the task must be Cancelled, not stuck Running.
    let tasks = store.list_subagent_tasks(&session_id).await.unwrap();
    assert_eq!(tasks.len(), 1, "expected exactly one subagent task");
    assert!(
        matches!(tasks[0].status, SubagentStatus::Cancelled),
        "task must be Cancelled after hard abort, got {:?}",
        tasks[0].status
    );
}

#[tokio::test]
async fn continue_after_hard_abort_does_not_hang() {
    let store = mem_store().await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![task_turn("explore something")])
            .push_script(vec![bash_call("sleep 30")])
            .push_script(vec![text_done("recovered")]),
    ) as Arc<dyn ChatStream>;

    let agent = resolve_agent("act").unwrap();
    let cancel = CancellationToken::new();
    let mut session = SessionState::new(
        "hard-abort-continue",
        agent,
        config(),
        mock,
        std::env::temp_dir(),
    )
    .with_cancel(cancel.clone())
    .with_store(store.clone());
    let session_id = session.id.clone();

    // ---- Run 1: dispatch a subagent, then hard-abort it mid-tool. ----
    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_for_wait = events.clone();
    let cancel_for_wait = cancel.clone();
    let waiter = tokio::spawn(async move {
        wait_for_subagent_start(&events_for_wait).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        cancel_for_wait.cancel();
    });

    let r1 = tokio::time::timeout(
        Duration::from_secs(15),
        run(&mut session, "go".into(), move |ev| {
            events.lock().unwrap().push(ev);
        }),
    )
    .await;
    let _ = waiter.await;
    assert!(
        r1.is_ok(),
        "first run did not complete within 15s; hard-abort during subagent is broken"
    );

    // ---- Run 2: a fresh turn must not hang on the stale child. ----
    // Give the session a fresh (uncancelled) token so the new turn can run;
    // the abandoned task is replayed-and-skipped (has_new_input) by run_loop.
    session = session.with_cancel(CancellationToken::new());
    let events2: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events2_sink = events2.clone();
    let start = Instant::now();
    let r2 = tokio::time::timeout(
        Duration::from_secs(15),
        run(&mut session, "again".into(), move |ev| {
            events2_sink.lock().unwrap().push(ev);
        }),
    )
    .await;
    let elapsed = start.elapsed();

    assert!(
        r2.is_ok(),
        "second run did not complete within 15s; continuing after abort is broken"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "second run took {elapsed:?}; expected a fast continue"
    );

    {
        let evs2 = events2.lock().unwrap();
        let saw_done = evs2.iter().any(|e| matches!(e, SessionEvent::Done));
        assert!(saw_done, "expected Done event after continuing post-abort");
    }

    // The old task must be terminal (not Running), so it never causes a 400.
    let tasks = store.list_subagent_tasks(&session_id).await.unwrap();
    for t in &tasks {
        assert!(
            !matches!(t.status, SubagentStatus::Running),
            "task {} must not be stuck Running after continue",
            t.task_id
        );
    }
}

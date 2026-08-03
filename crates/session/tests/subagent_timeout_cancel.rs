//! Integration tests for the subagent idle-timeout watchdog.
//!
//! `task_timeout_secs` bounds a *single stalled step*, not total runtime:
//! every child event (tool start/end, LLM text/reasoning deltas) resets the
//! deadline in the Phase-1 loop, so an active subagent runs indefinitely while
//! a wedged step trips after `task_timeout_secs` of silence.
//!
//! - `timeout_marks_subagent_cancelled`: a stalled step (bash that does not
//!   return) trips the idle deadline; the task ends up Cancelled, and the
//!   child's hard-cancel token is fired so its cleanup runs inside the grace
//!   drain (DB overridden to Cancelled, not Completed).
//! - `sustained_activity_does_not_timeout`: a child that keeps producing
//!   events (tool calls every < task_timeout) for longer than task_timeout is
//!   NOT killed — the key regression proving the semantics changed from a
//!   single wall-clock cap to a per-step idle timeout.
//! - `stalled_single_step_times_out`: a single long bash call with no
//!   intermediate events trips the idle deadline promptly (< the bash
//!   backgrounding point), surfacing a timeout to the parent.

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

/// A child that keeps making progress (tool calls every ~0.2s) must NOT be
/// killed even though its total runtime (~1.2s) exceeds the 1s task_timeout.
/// Each tool start/end resets the idle deadline; only a truly stalled step
/// trips it. This is the key regression proving the semantics changed from a
/// single wall-clock cap to a per-step idle timeout: under the OLD semantics
/// this run would have been killed at 1s.
#[tokio::test]
async fn sustained_activity_does_not_timeout() {
    let store = mem_store().await;
    // Six short bash calls (~0.2s each). Total runtime ~1.2s > 1s timeout, but
    // every idle gap (the bash execution window) is ~0.2s << 1s, so the
    // deadline keeps resetting and never fires.
    let mut builder = MockChatClient::new().push_script(vec![task_turn("explore with many steps")]);
    for _ in 0..6 {
        builder = builder.push_script(vec![bash_call("sleep 0.2")]);
    }
    let mock = Arc::new(builder.with_default(vec![text_done("explored everything")]))
        as Arc<dyn ChatStream>;

    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        "sustained-activity-test",
        agent,
        config(),
        mock,
        std::env::temp_dir(),
    )
    .with_store(store.clone());
    let session_id = session.id.clone();

    let started = std::time::Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        run(&mut session, "go".into(), |_| {}),
    )
    .await;
    let elapsed = started.elapsed();
    assert!(
        result.is_ok(),
        "run did not complete within 30s; active subagent was likely killed"
    );
    // Must have run past the 1s timeout (proving it survived the old cap).
    assert!(
        elapsed >= Duration::from_secs(1),
        "expected sustained run > 1s, got {:?}",
        elapsed
    );

    let tasks = store.list_subagent_tasks(&session_id).await.unwrap();
    assert_eq!(tasks.len(), 1, "expected exactly one subagent task");
    assert!(
        matches!(tasks[0].status, SubagentStatus::Completed),
        "an active subagent must complete, not be Cancelled; got {:?}",
        tasks[0].status
    );
}

/// A single bash call that stalls (no intermediate events) must trip the idle
/// deadline. Under cfg(test) bash backgrounds at 1s; with a 1s task_timeout the
/// idle deadline (reset at ToolStart) fires during the execution window, so the
/// subagent is killed promptly rather than waiting for the command to finish.
#[tokio::test]
async fn stalled_single_step_times_out() {
    let store = mem_store().await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![task_turn("run a slow command")])
            .push_script(vec![bash_call("sleep 30")])
            .with_default(vec![text_done("done")]),
    ) as Arc<dyn ChatStream>;

    let agent = resolve_agent("act").unwrap();
    let mut session = SessionState::new(
        "stalled-step-test",
        agent,
        config(),
        mock,
        std::env::temp_dir(),
    )
    .with_store(store.clone());
    let session_id = session.id.clone();

    let started = std::time::Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        run(&mut session, "go".into(), |_| {}),
    )
    .await;
    let elapsed = started.elapsed();
    assert!(
        result.is_ok(),
        "run did not complete within 30s; stalled-step timeout drain is broken"
    );
    // The timeout must fire ~1s after the bash ToolStart — well before the
    // 30s sleep would finish (or even its 1s backgrounding + drain). This
    // bounds the kill to a prompt window.
    assert!(
        elapsed < Duration::from_secs(15),
        "stalled-step timeout fired too late ({:?}); deadline reset may be broken",
        elapsed
    );

    let tasks = store.list_subagent_tasks(&session_id).await.unwrap();
    assert_eq!(tasks.len(), 1, "expected exactly one subagent task");
    assert!(
        matches!(tasks[0].status, SubagentStatus::Cancelled),
        "a stalled subagent must be Cancelled after idle timeout, got {:?}",
        tasks[0].status
    );
}

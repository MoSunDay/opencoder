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
//!   events (LLM text deltas every < task_timeout) for longer than task_timeout is
//!   NOT killed — the key regression proving the semantics changed from a
//!   single wall-clock cap to a per-step idle timeout.
//! - `stalled_single_step_times_out`: a single long bash call with no
//!   intermediate events trips the idle deadline promptly (< the bash
//!   backgrounding point), surfacing a timeout to the parent.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatRequest, ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
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

/// Only the child stream uses delayed events. No shell process startup or
/// filesystem / database latency is part of the idle-watchdog clock.
struct ActiveChild {
    parent: MockChatClient,
    calls: std::sync::atomic::AtomicUsize,
}

impl ChatStream for ActiveChild {
    fn chat_stream(
        &self,
        req: ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) != 1 {
            return self.parent.chat_stream(req);
        }
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move {
            for _ in 0..6 {
                tokio::time::sleep(Duration::from_millis(200)).await;
                if tx
                    .send(LlmEvent::TextDelta("progress".into()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            let _ = tx.send(text_done("explored everything")).await;
        });
        Ok(rx)
    }
}

/// Progress every 200ms must survive a 1s idle budget for a 1.2s run.
/// Virtual time keeps this assertion independent of CI host scheduling.
#[tokio::test(start_paused = true)]
async fn sustained_activity_does_not_timeout() {
    let mock = Arc::new(ActiveChild {
        parent: MockChatClient::new()
            .push_script(vec![task_turn("explore with sustained progress")])
            .with_default(vec![text_done("done")]),
        calls: std::sync::atomic::AtomicUsize::new(0),
    }) as Arc<dyn ChatStream>;
    let mut session = SessionState::new(
        "sustained-activity-test",
        resolve_agent("act").unwrap(),
        config(),
        mock,
        std::env::temp_dir(),
    );
    let started = tokio::time::Instant::now();
    let mut events = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(30),
        run(&mut session, "go".into(), |event| events.push(event)),
    )
    .await
    .expect("active subagent must finish")
    .expect("parent run must succeed");
    assert!(started.elapsed() >= Duration::from_millis(1200));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event,
                SessionEvent::SubagentChild { ev, .. }
                    if matches!(ev.as_ref(), SessionEvent::TextDelta(_))
            ))
            .count(),
        6,
        "all six child progress events must reach the parent"
    );
    assert!(
        events.iter().any(|event| matches!(event,
            SessionEvent::SubagentEnd { ok: true, cancelled: false, summary, .. }
                if summary == &format!("(0 tool calls) {}", "progress".repeat(6))
        )),
        "active child must finish successfully after exceeding its idle budget"
    );
    assert!(
        events.iter().any(|event| matches!(event,
            SessionEvent::ToolEnd { name, output, is_error: false, .. }
                if name == "task" && output == "explored everything"
        )),
        "completed child result must reach the parent task output"
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

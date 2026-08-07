//! D1 regression: when a subagent TIMES OUT (idle deadline elapses), the
//! terminal `SubagentEnd` event and the parent-facing error must report a
//! *timeout*, not "redirected by parent steer".
//!
//! Before the fix, execute.rs fired the child's hard-cancel token itself on
//! timeout, so in `run_subagent`'s post-run check `child.cancel.is_cancelled()`
//! was true while `parent.cancel` was intact — indistinguishable from a real
//! parent steer. The shared `timed_out` flag now disambiguates the two.
//!
//! This test uses a child LLM stream that produces NO events (never resolves),
//! so no activity resets the idle deadline and the timeout fires reliably
//! (avoiding the bash-backgrounding timing race).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatRequest, ChatStream, CompletedToolCall, LlmEvent, Usage};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, Store, SubagentStatus};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config() -> Config {
    Config {
        model: "m/g".into(),
        // 1s idle deadline: the child produces no events, so this trips after
        // ~1s of silence.
        task_timeout_secs: Some(1),
        // Generous drain so the child finishes its cleanup *inside* the grace
        // window (the `Ok(o)` override path).
        subagent_drain_secs: Some(15),
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

/// A stream whose FIRST call (the parent's turn) dispatches a `task` subagent
/// immediately. The SECOND call (the child's first turn) never resolves: the
/// sender is held but never sends, so `rx.recv()` blocks forever and the child
/// produces no activity, letting the parent's idle timeout fire. Any later
/// call (the parent's follow-up after the failed subagent) completes with a
/// plain text turn so the run finishes instead of hanging.
struct TaskThenBlockStream {
    calls: AtomicUsize,
}

impl TaskThenBlockStream {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl ChatStream for TaskThenBlockStream {
    fn chat_stream(&self, _req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel::<LlmEvent>(128);
        match n {
            0 => {
                // Parent: dispatch the subagent right away.
                tokio::spawn(async move {
                    let _ = tx.send(task_turn("explore something")).await;
                });
            }
            1 => {
                // Child: never produce events. Held alive by the pending task
                // so `rx.recv()` never resolves; the idle deadline in
                // execute.rs fires after task_timeout_secs.
                tokio::spawn(async move {
                    std::future::pending::<()>().await;
                    drop(tx);
                });
            }
            _ => {
                // Parent follow-up after the timed-out subagent: complete so
                // the run finishes instead of hanging.
                tokio::spawn(async move {
                    let _ = tx
                        .send(text_done("subagent was interrupted, stopping"))
                        .await;
                });
            }
        }
        Ok(rx)
    }
}

#[tokio::test]
async fn subagent_timeout_reports_timeout_not_steer() {
    let store = mem_store().await;
    let mock = Arc::new(TaskThenBlockStream::new()) as Arc<dyn ChatStream>;

    let agent = resolve_agent("act").unwrap();
    let cancel = CancellationToken::new();
    let mut session = SessionState::new(
        "d1-timeout-summary",
        agent,
        config(),
        mock,
        std::env::temp_dir(),
    )
    .with_store(store.clone())
    .with_cancel(cancel);
    let session_id = session.id.clone();

    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();

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

    // The subagent task must be Cancelled.
    let tasks = store.list_subagent_tasks(&session_id).await.unwrap();
    assert_eq!(tasks.len(), 1, "expected exactly one subagent task");
    assert!(
        matches!(tasks[0].status, SubagentStatus::Cancelled),
        "task must be Cancelled after timeout, got {:?}",
        tasks[0].status
    );

    // The terminal SubagentEnd event must report a TIMEOUT, not a steer.
    let end = events
        .lock()
        .unwrap()
        .iter()
        .find(|ev| matches!(ev, SessionEvent::SubagentEnd { .. }))
        .cloned();
    let summary = match end {
        Some(SessionEvent::SubagentEnd {
            summary, cancelled, ..
        }) => {
            assert!(cancelled, "SubagentEnd must be marked cancelled");
            summary
        }
        other => panic!("expected a SubagentEnd event, got {:?}", other),
    };
    let lower = summary.to_lowercase();
    assert!(
        lower.contains("timed out") || lower.contains("timeout"),
        "SubagentEnd summary must mention a timeout, got: {summary:?}"
    );
    assert!(
        !lower.contains("steer") && !lower.contains("redirected"),
        "SubagentEnd summary must NOT mention a parent steer (D1 bug), got: {summary:?}"
    );
}

//! Unit tests for [`super`] (tool-call execution), split out of the
//! source file to respect its line budget.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::json;

use super::*;
use crate::SessionEvent;
use opencoder_core::{resolve_agent, Config, Tool, ToolContext, ToolOutput};
use opencoder_llm::{ChatStream, MockChatClient};

/// A tool whose `execute` future never resolves, to exercise the timeout
/// safety net without depending on a real long-running tool.
struct HangingTool;

#[async_trait]
impl Tool for HangingTool {
    fn name(&self) -> &str {
        "hang"
    }
    fn description(&self) -> &str {
        "never resolves"
    }
    fn parameters(&self) -> serde_json::Value {
        json!({})
    }
    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        std::future::pending::<()>().await;
        unreachable!()
    }
}

/// A tool that resolves instantly, to confirm the timeout does not trip
/// for well-behaved tools.
struct FastTool;

#[async_trait]
impl Tool for FastTool {
    fn name(&self) -> &str {
        "fast"
    }
    fn description(&self) -> &str {
        "resolves immediately"
    }
    fn parameters(&self) -> serde_json::Value {
        json!({})
    }
    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::ok("done"))
    }
}

fn make_session() -> SessionState {
    SessionState::new(
        "sess-test",
        resolve_agent("act").unwrap(),
        Config::default(),
        Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
        std::env::temp_dir().join("opencer-execute-tests"),
    )
}

#[tokio::test]
async fn hung_tool_returns_timeout_error() {
    let session = make_session();
    let registry: HashMap<String, ToolArc> =
        [("hang".to_string(), Arc::new(HangingTool) as ToolArc)]
            .into_iter()
            .collect();
    let mut noop: Box<dyn FnMut(SessionEvent) + Send> = Box::new(|_| {});
    let sink: Sink<'_> = Arc::new(Mutex::new(&mut *noop));
    let tc = CompletedToolCall {
        id: "tc-1".into(),
        name: "hang".into(),
        input: json!({}),
    };
    let out = execute_call_with_timeout(
        &tc,
        &session,
        &registry,
        &sink,
        Some(Duration::from_millis(50)),
    )
    .await;
    assert!(out.is_error);
    assert!(
        out.content.contains("timed out"),
        "expected timeout message, got: {}",
        out.content
    );
}

#[tokio::test]
async fn fast_tool_is_unaffected_by_timeout() {
    let session = make_session();
    let registry: HashMap<String, ToolArc> = [("fast".to_string(), Arc::new(FastTool) as ToolArc)]
        .into_iter()
        .collect();
    let mut noop: Box<dyn FnMut(SessionEvent) + Send> = Box::new(|_| {});
    let sink: Sink<'_> = Arc::new(Mutex::new(&mut *noop));
    let tc = CompletedToolCall {
        id: "tc-2".into(),
        name: "fast".into(),
        input: json!({}),
    };
    // A short timeout that would trip if the tool hung; a fast tool must
    // still return its real result, not the timeout error.
    let out = execute_call_with_timeout(
        &tc,
        &session,
        &registry,
        &sink,
        Some(Duration::from_secs(30)),
    )
    .await;
    assert!(!out.is_error);
    assert_eq!(out.content, "done");
}

/// A tool that wraps synchronous blocking work in `spawn_blocking` and
/// sleeps ~200ms, simulating a real search/ls directory scan. Exercises
/// the turn_cancel interrupt path: with `spawn_blocking` the async worker
/// thread is free to poll the cancel token, so firing turn_cancel mid-scan
/// produces "turn interrupted" rather than blocking until completion.
struct BlockingTool;

#[async_trait]
impl Tool for BlockingTool {
    fn name(&self) -> &str {
        "block"
    }
    fn description(&self) -> &str {
        "blocks for 200ms in spawn_blocking"
    }
    fn parameters(&self) -> serde_json::Value {
        json!({})
    }
    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let out = tokio::task::spawn_blocking(move || -> ToolOutput {
            std::thread::sleep(std::time::Duration::from_millis(200));
            ToolOutput::ok("blocking done")
        })
        .await
        .unwrap_or_else(|e| ToolOutput::err(format!("blocking task failed: {e}")));
        Ok(out)
    }
}

// turn_cancel must interrupt a spawn_blocking tool mid-execution. Before the
// search/ls fix, synchronous tools hijacked the worker thread and the
// cancel token was never polled. With spawn_blocking the blocking work runs
// on a dedicated thread, so the async select! can poll turn_cancel and win.
#[tokio::test]
async fn turn_cancel_interrupts_blocking_tool() {
    let session = make_session();
    let registry: HashMap<String, ToolArc> =
        [("block".to_string(), Arc::new(BlockingTool) as ToolArc)]
            .into_iter()
            .collect();
    let mut noop: Box<dyn FnMut(SessionEvent) + Send> = Box::new(|_| {});
    let sink: Sink<'_> = Arc::new(Mutex::new(&mut *noop));
    let tc = CompletedToolCall {
        id: "tc-block".into(),
        name: "block".into(),
        input: json!({}),
    };

    // Grab the turn_cancel token before executing.
    let turn_cancel = session.turn_cancel.as_ref().unwrap().clone();

    // Fire turn_cancel concurrently after a short delay (well within the
    // 200ms blocking window).
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        if let Ok(g) = turn_cancel.lock() {
            g.cancel();
        }
    });

    let out = execute_call_with_timeout(
        &tc,
        &session,
        &registry,
        &sink,
        // Generous timeout so only turn_cancel can win.
        Some(Duration::from_secs(10)),
    )
    .await;

    assert!(out.is_error);
    assert!(
        out.content.contains("turn interrupted"),
        "expected 'turn interrupted', got: {}",
        out.content
    );
}

// The blocking tool must still complete normally when no cancel fires —
// spawn_blocking does not break the happy path.
#[tokio::test]
async fn blocking_tool_completes_when_not_interrupted() {
    let session = make_session();
    let registry: HashMap<String, ToolArc> =
        [("block".to_string(), Arc::new(BlockingTool) as ToolArc)]
            .into_iter()
            .collect();
    let mut noop: Box<dyn FnMut(SessionEvent) + Send> = Box::new(|_| {});
    let sink: Sink<'_> = Arc::new(Mutex::new(&mut *noop));
    let tc = CompletedToolCall {
        id: "tc-block-ok".into(),
        name: "block".into(),
        input: json!({}),
    };

    let out = execute_call_with_timeout(
        &tc,
        &session,
        &registry,
        &sink,
        Some(Duration::from_secs(10)),
    )
    .await;

    assert!(!out.is_error);
    assert_eq!(out.content, "blocking done");
}
/// With `timeout: None` the safety net must never fire: a perpetually-pending
/// tool stays pending (responds only to a cancel), rather than erroring with a
/// "timed out" message. This is the bash exemption — bash has its own internal
/// timeout (BASH_TIMEOUT_SECS) and does not rely on this safety net.
#[tokio::test]
async fn none_timeout_never_fires_for_hung_tool() {
    let session = make_session();
    let registry: HashMap<String, ToolArc> =
        [("hang".to_string(), Arc::new(HangingTool) as ToolArc)]
            .into_iter()
            .collect();
    let mut noop: Box<dyn FnMut(SessionEvent) + Send> = Box::new(|_| {});
    let sink: Sink<'_> = Arc::new(Mutex::new(&mut *noop));
    let tc = CompletedToolCall {
        id: "tc-3".into(),
        name: "hang".into(),
        input: json!({}),
    };
    let call = execute_call_with_timeout(&tc, &session, &registry, &sink, None);
    // The call should NOT resolve on its own (no deadline, hung tool). Race it
    // against a short outer deadline and confirm it was still pending.
    if let Ok(out) = tokio::time::timeout(Duration::from_millis(120), call).await {
        panic!("None deadline should never fire; got: {}", out.content);
    }
}

/// `force_cancel_subagent` (the grace-expiry fallback) must replicate the
/// critical side-effects of `run_subagent`'s cleanup: mark the DB task
/// Cancelled, prune the stale registry entries, and emit SubagentEnd.
#[tokio::test]
async fn force_cancel_marks_task_and_prunes_registries() {
    use opencoder_store::{LibsqlStore, Store, SubagentStatus, SubagentTaskRecord};

    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&opencoder_store::SessionMeta {
            id: "force-cancel-parent".into(),
            title: None,
            agent: Some("act".into()),
            model: Some("m".into()),

            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();
    store
        .create_session(&opencoder_store::SessionMeta {
            id: "child-1".into(),
            title: None,
            agent: Some("explore".into()),
            model: Some("m".into()),

            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();
    store
        .create_subagent_task(&SubagentTaskRecord {
            task_id: "call-1".into(),
            parent_session_id: "force-cancel-parent".into(),
            child_session_id: "child-1".into(),
            parent_message_id: None,
            agent: "explore".into(),
            prompt: "x".into(),
            result: None,
            status: SubagentStatus::Running,
            ok: None,
            started_at: 0,
            completed_at: None,
        })
        .await
        .unwrap();

    // Build a session with the store and pre-populated registry entries.
    let mut session = make_session();
    session = session.with_store(store.clone());
    let call_id = "call-1";
    let token = CancellationToken::new();
    session
        .child_cancels
        .lock()
        .unwrap()
        .insert(call_id.to_string(), token);
    session
        .child_turn_cancels
        .lock()
        .unwrap()
        .insert(call_id.to_string(), {
            Arc::new(Mutex::new(CancellationToken::new()))
        });
    session
        .child_steer_gates
        .lock()
        .unwrap()
        .insert(call_id.to_string(), crate::SubagentSteerGate::new());

    let mut noop: Box<dyn FnMut(SessionEvent) + Send> = Box::new(|_| {});
    let sink: Sink<'_> = Arc::new(Mutex::new(&mut *noop));

    force_cancel_subagent(
        session.store.clone(),
        session.child_cancels.clone(),
        session.child_turn_cancels.clone(),
        session.child_steer_gates.clone(),
        &sink,
        call_id,
    )
    .await;

    // DB task must be Cancelled.
    let tasks = store
        .list_subagent_tasks("force-cancel-parent")
        .await
        .unwrap();
    store
        .create_session(&opencoder_store::SessionMeta {
            id: "child-1".into(),
            title: None,
            agent: Some("explore".into()),
            model: Some("m".into()),

            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(
        matches!(tasks[0].status, SubagentStatus::Cancelled),
        "task must be Cancelled after force_cancel, got {:?}",
        tasks[0].status
    );
    // Registries must be pruned.
    assert!(
        session.child_cancels.lock().unwrap().is_empty(),
        "child_cancels must be empty after force_cancel"
    );
    assert!(
        session.child_turn_cancels.lock().unwrap().is_empty(),
        "child_turn_cancels must be empty after force_cancel"
    );
    assert!(
        session.child_steer_gates.lock().unwrap().is_empty(),
        "child_steer_gates must be empty after force_cancel"
    );
}

#[tokio::test]
async fn sidecar_cannot_farm_mutations_out_via_task() {
    // rules/01 regression (brief #2): the generic gate never sees `task`
    // (it early-returns above), so a sidecar could spawn a full
    // write-capable subagent and mutate the repo through it. The spawn
    // must be denied with the standard sidecar denial, before any child
    // session is created.
    let session = SessionState::new(
        "sess-sidecar-task",
        resolve_agent("sidecar").unwrap(),
        Config::default(),
        Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
        std::env::temp_dir().join("opencoder-execute-tests"),
    );
    let registry: HashMap<String, ToolArc> = HashMap::new();
    let mut noop: Box<dyn FnMut(SessionEvent) + Send> = Box::new(|_| {});
    let sink: Sink<'_> = Arc::new(Mutex::new(&mut *noop));
    let tc = CompletedToolCall {
        id: "tc-sidecar".into(),
        name: "task".into(),
        input: json!({
            "description": "edit files",
            "prompt": "mutate the working tree",
            "agent": "build"
        }),
    };
    let out = execute_call_with_timeout(&tc, &session, &registry, &sink, None).await;
    assert!(
        out.is_error,
        "sidecar task spawn must be denied, got: {}",
        out.content
    );
    assert!(
        out.content.contains("Blocked in sidecar"),
        "expected the sidecar denial, got: {}",
        out.content
    );
}

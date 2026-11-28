//! Shared fixtures for the TODO runtime integration suites.
//!
//! Kept in a separate module so each consuming test file
//! (`interrupt.rs`, `interrupt_retry.rs`, `late_results.rs`,
//! `interrupt_intent.rs`) stays within the per-file line budget without
//! duplicating the durable-store poll loops that observe batch dispatch and
//! child-session transcripts, or the scripted-reply / one-todo / runtime
//! arrangements the lock-outside interrupt suites vary.

#![allow(dead_code)] // each consuming test target uses a different subset

use std::sync::Arc;
use std::time::{Duration, Instant};

use opencoder_core::Config;
use opencoder_llm::{ChatStream, LlmEvent};
use opencoder_store::Store;
use opencoder_todos::{domain, parent, persistence, types::*, Runtime};
use tokio_util::sync::CancellationToken;

/// Poll the durable item projections until the TODO shows as running (the
/// batch committed "todos_dispatched" and execution is in flight).
pub async fn wait_until_running(store: &Arc<dyn Store>, workflow_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let items = store.list_todo_items(workflow_id).await.unwrap();
        if items.iter().any(|item| item.status == "running") {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "todo in {workflow_id} never reached running"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Wait until the given TODO's child session has produced an assistant
/// message containing `marker` (i.e. the sibling's execution has finished
/// recording its candidate while the parked TODO is still in flight).
pub async fn wait_for_session_message(
    store: &Arc<dyn Store>,
    workflow_id: &str,
    todo_id: &str,
    marker: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let session_id = store
            .list_todo_items(workflow_id)
            .await
            .unwrap()
            .into_iter()
            .find(|item| item.todo_id == todo_id)
            .and_then(|item| item.active_session_id);
        if let Some(session_id) = session_id {
            let transcript = store
                .load_messages(&session_id)
                .await
                .unwrap()
                .iter()
                .map(|message| message.text())
                .collect::<String>();
            if transcript.contains(marker) {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "TODO {todo_id} never produced the marker message"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// ---------------------------------------------------------------------------
// Lock-outside interrupt fixtures (interrupt_intent.rs). Plain functions over
// values: a scripted reply, a one-todo spec, a runtime and a persisted
// workflow, so a test only states the arrangement it varies.
// ---------------------------------------------------------------------------

/// A completed assistant reply carrying exactly `text`.
pub fn done(text: &str) -> Vec<LlmEvent> {
    vec![LlmEvent::Completed {
        text: text.into(),
        tool_calls: Vec::new(),
        usage: None,
    }]
}

/// A parent decision dispatching `todo_id` into a `context_mode` session.
pub fn dispatch(todo_id: &str, context_mode: &str) -> Vec<LlmEvent> {
    done(&format!(
        r#"{{"operation":"dispatch","todos":[{{"todo_id":"{todo_id}","context_mode":"{context_mode}"}}],"reason":"ready"}}"#
    ))
}

/// A valid one-todo (`step-1`, agent `act`) spec with workflow id `id`.
pub fn one_todo_spec(id: &str, objective: &str) -> WorkflowSpec {
    opencoder_todos::parse_spec(&format!(
        r#"{{
            "schema_version": 1,
            "id": "{id}",
            "name": "intent test",
            "objective": "{objective}",
            "constraints": [],
            "todos": [{{
                "id": "step-1",
                "title": "step",
                "requirement_background": "required by test",
                "instructions": "return the candidate",
                "depends_on": [],
                "agent": "act",
                "max_attempts": 2,
                "acceptance": {{"criteria": "candidate exists", "required_tool_calls": []}}
            }}],
            "metadata": null
        }}"#
    ))
    .unwrap()
}

/// A runtime over `store` / `client` rooted at `dir`, without debug dumps.
pub fn runtime(
    store: &Arc<dyn Store>,
    client: Arc<dyn ChatStream>,
    dir: &std::path::Path,
) -> Runtime {
    Runtime {
        store: store.clone(),
        client,
        config: Config::default(),
        workdir: dir.to_path_buf(),
        debug_root: None,
        cancel: CancellationToken::new(),
    }
}

/// Persist workflow `id` in its initial (Pending) state with its parent
/// session — the rows `Runtime::run_new_with_id` creates before the first
/// transition. Returns the spec and the persisted state.
pub async fn seeded(store: &Arc<dyn Store>, id: &str) -> (WorkflowSpec, WorkflowState) {
    let spec = one_todo_spec(id, "verify the lock-outside interrupt contract");
    let state = domain::initial_state(&spec, id.into(), "parent-1".into());
    parent::create_session(store, &state, &Config::default())
        .await
        .unwrap();
    persistence::create(store, &spec, &state).await.unwrap();
    (spec, state)
}

/// The persisted state of workflow `id`.
pub async fn state(store: &Arc<dyn Store>, id: &str) -> WorkflowState {
    persistence::load(store, id)
        .await
        .unwrap()
        .expect("workflow must exist")
        .1
}

/// Kinds of every transition event of workflow `id`, oldest first.
pub async fn kinds(store: &Arc<dyn Store>, id: &str) -> Vec<String> {
    store
        .todo_events_after(id, 0)
        .await
        .unwrap()
        .into_iter()
        .map(|event| event.kind)
        .collect()
}

/// Assert the workflow's interruption event records `request_id` and return
/// the number of interruption events (a request is committed exactly once).
pub async fn assert_interrupted_with(store: &Arc<dyn Store>, id: &str, request_id: &str) -> usize {
    let events = store.todo_events_after(id, 0).await.unwrap();
    let interruptions = events
        .iter()
        .filter(|event| event.kind == "workflow_interrupted")
        .collect::<Vec<_>>();
    assert!(
        interruptions
            .iter()
            .any(|event| { event.payload["request_id"].as_str() == Some(request_id) }),
        "no interruption event carries {request_id}: {interruptions:?}"
    );
    interruptions.len()
}

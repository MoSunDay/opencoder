//! Shared polling helpers for the TODO runtime integration suites.
//!
//! Kept in a separate module so each consuming test file
//! (`interrupt.rs`, `interrupt_retry.rs`, `late_results.rs`) stays within
//! the per-file line budget without duplicating the durable-store poll
//! loops that observe batch dispatch and child-session transcripts.

#![allow(dead_code)] // each consuming test target uses a different subset

use std::sync::Arc;
use std::time::{Duration, Instant};

use opencoder_store::Store;

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

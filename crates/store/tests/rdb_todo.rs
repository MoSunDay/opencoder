//! Todo-workflow contracts for the rdb (RocksDB) backend, ported from the
//! SQL suite: atomic versioned transitions with generation conflicts,
//! per-workflow event cursors (forward, reverse, byte-budgeted), keyset
//! item pagination, bounded field reads and session cascade deletes.
//! Transition, event-cursor and listing suites live in `rdb_todo/`.

#[path = "rdb_todo/suite_event_cursors.rs"]
mod suite_event_cursors;
#[path = "rdb_todo/suite_listings.rs"]
mod suite_listings;
#[path = "rdb_todo/suite_transitions.rs"]
mod suite_transitions;

use opencoder_store::{
    SessionMeta, Store, TodoEventRecord, TodoItemRecord, TodoWorkflowRecord, TodoWorkflowSummary,
};

pub(crate) fn parent(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        title: Some("workflow".into()),
        created_at: 1,
        updated_at: 1,
        ..Default::default()
    }
}

pub(crate) fn workflow(id: &str, generation: i64, status: &str) -> TodoWorkflowRecord {
    TodoWorkflowRecord {
        id: id.into(),
        parent_session_id: "parent-1".into(),
        status: status.into(),
        spec_json: serde_json::json!({ "id": id }),
        state_json: serde_json::json!({ "generation": generation }),
        generation,
        created_at: 1,
        updated_at: generation + 1,
        terminal_reason: None,
    }
}

pub(crate) fn item(workflow: &str, todo_id: &str, ordinal: i64, status: &str) -> TodoItemRecord {
    TodoItemRecord {
        workflow_id: workflow.into(),
        todo_id: todo_id.into(),
        ordinal,
        status: status.into(),
        attempt: 0,
        active_session_id: None,
        session_history: vec![format!("sess-{todo_id}")],
        result_json: None,
        last_error: None,
        updated_at: 1,
    }
}

pub(crate) fn event(workflow: &str, kind: &str) -> TodoEventRecord {
    TodoEventRecord {
        seq: None,
        workflow_id: workflow.into(),
        kind: kind.into(),
        payload: serde_json::json!({}),
        ts: 1,
    }
}

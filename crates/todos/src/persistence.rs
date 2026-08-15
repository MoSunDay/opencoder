use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use opencoder_store::{
    Store, TodoEventRecord, TodoItemRecord, TodoWorkflowRecord, TodoWorkflowSummary,
};

use crate::types::{WorkflowSpec, WorkflowState};

pub async fn create(
    store: &Arc<dyn Store>,
    spec: &WorkflowSpec,
    state: &WorkflowState,
) -> Result<i64> {
    let now = opencoder_core::message::now_ms();
    store
        .create_todo_workflow(
            &workflow_record(spec, state, now)?,
            &item_records(spec, state, now)?,
            &TodoEventRecord {
                seq: None,
                workflow_id: state.workflow_id.clone(),
                kind: "workflow_created".into(),
                payload: serde_json::json!({"state": state}),
                ts: now,
            },
        )
        .await
}

pub async fn commit(
    store: &Arc<dyn Store>,
    spec: &WorkflowSpec,
    state: &WorkflowState,
    kind: &str,
    payload: serde_json::Value,
) -> Result<i64> {
    let now = opencoder_core::message::now_ms();
    store
        .commit_todo_transition(
            &workflow_record(spec, state, now)?,
            &item_records(spec, state, now)?,
            &TodoEventRecord {
                seq: None,
                workflow_id: state.workflow_id.clone(),
                kind: kind.into(),
                payload,
                ts: now,
            },
        )
        .await
}

pub async fn load(
    store: &Arc<dyn Store>,
    id: &str,
) -> Result<Option<(WorkflowSpec, WorkflowState)>> {
    let Some(record) = store.get_todo_workflow(id).await? else {
        return Ok(None);
    };
    let spec = serde_json::from_value(record.spec_json).context("decode workflow spec")?;
    let state = serde_json::from_value(record.state_json).context("decode workflow state")?;
    Ok(Some((spec, state)))
}

pub async fn list(store: &Arc<dyn Store>) -> Result<Vec<TodoWorkflowSummary>> {
    store.list_todo_workflows(100).await
}

fn workflow_record(
    spec: &WorkflowSpec,
    state: &WorkflowState,
    now: i64,
) -> Result<TodoWorkflowRecord> {
    Ok(TodoWorkflowRecord {
        id: state.workflow_id.clone(),
        parent_session_id: state.parent_session_id.clone(),
        status: state.status.as_str().into(),
        spec_json: serde_json::to_value(spec)?,
        state_json: serde_json::to_value(state)?,
        generation: state.generation as i64,
        created_at: now,
        updated_at: now,
        terminal_reason: state.terminal_reason.clone(),
    })
}

fn item_records(
    spec: &WorkflowSpec,
    state: &WorkflowState,
    now: i64,
) -> Result<Vec<TodoItemRecord>> {
    spec.todos
        .iter()
        .enumerate()
        .map(|(ordinal, spec_todo)| {
            let item = &state.todos[&spec_todo.id];
            Ok(TodoItemRecord {
                workflow_id: state.workflow_id.clone(),
                todo_id: spec_todo.id.clone(),
                ordinal: ordinal as i64 + 1,
                status: item.status.as_str().into(),
                attempt: i64::from(item.attempt),
                active_session_id: item.active_session_id.clone(),
                session_history: item.session_history.clone(),
                result_json: item
                    .candidate
                    .as_ref()
                    .map(serde_json::to_value)
                    .transpose()?,
                last_error: item.last_error.clone(),
                updated_at: now,
            })
        })
        .collect()
}

pub async fn debug_dump(
    store: &Arc<dyn Store>,
    spec: &WorkflowSpec,
    state: &WorkflowState,
    root: &Path,
) -> Result<()> {
    let base = root.join(&state.workflow_id);
    write_json(&base.join("task-info/workflow.json"), spec).await?;
    write_json(&base.join("task-info/index.json"), state).await?;
    for todo in &spec.todos {
        write_json(
            &base
                .join("task-info/todos")
                .join(format!("{}.json", todo.id)),
            &serde_json::json!({"spec":todo,"state":state.todos[&todo.id]}),
        )
        .await?;
    }
    let events = store.todo_events_after(&state.workflow_id, 0).await?;
    let ndjson = events
        .iter()
        .map(serde_json::to_string)
        .collect::<std::result::Result<Vec<_>, _>>()?
        .join("\n");
    atomic_write(
        &base.join("process/workflow/events.ndjson"),
        ndjson.as_bytes(),
    )
    .await?;
    write_json(
        &base.join("sessions/parent.json"),
        &serde_json::json!({"session_id":state.parent_session_id}),
    )
    .await?;
    for (todo_id, todo) in &state.todos {
        for (index, session_id) in todo.session_history.iter().enumerate() {
            write_json(
                &base
                    .join("sessions/todos")
                    .join(todo_id)
                    .join(format!("attempt-{:03}.json", index + 1)),
                &serde_json::json!({"session_id":session_id,"status":todo.status,"summary":todo.candidate.as_ref().map(|c| &c.summary)}),
            )
            .await?;
        }
    }
    Ok(())
}

async fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    atomic_write(path, &serde_json::to_vec_pretty(value)?).await
}

async fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let parent = path.parent().context("debug dump path has no parent")?;
    tokio::fs::create_dir_all(parent).await?;
    let temp = path.with_extension("tmp");
    tokio::fs::write(&temp, content).await?;
    tokio::fs::rename(&temp, path).await?;
    Ok(())
}

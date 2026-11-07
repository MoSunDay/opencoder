//! Todo workflow projections (`twf/<id>`).
//!
//! `create` mirrors an SQL INSERT (duplicate id rejects with the same
//! UNIQUE-constraint text); `commit` is the optimistic-generation update
//! (workflow + items + event in ONE batch). Workflows are NOT cascaded by
//! session deletes in this phase (the SQL FK exists but no runtime path
//! deletes a workflow's parent session while the workflow is live).

use anyhow::{bail, Result};

use crate::rdb_store::kv::{get_json, workflow_key};
use crate::rdb_store::tx;
use crate::{
    TodoEventRecord, TodoItemRecord, TodoWorkflowDetail, TodoWorkflowRecord, TodoWorkflowSummary,
};

pub fn get(store: &rdb::store::Store, id: &str) -> Result<Option<TodoWorkflowRecord>> {
    get_json(store, &workflow_key(id))
}

pub fn create(
    store: &rdb::store::Store,
    workflow: &TodoWorkflowRecord,
    items: &[TodoItemRecord],
    event: &TodoEventRecord,
) -> Result<i64> {
    let key = workflow_key(&workflow.id);
    if rdb::store::get(store, b"", &key)
        .map_err(crate::rdb_store::kv::engine_err)?
        .is_some()
    {
        bail!("insert todo workflow: UNIQUE constraint failed: todo_workflows.id");
    }
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &key, workflow)?;
    super::items::stage_upsert(&mut batch, items);
    let seq = super::events::stage_append(store, &mut batch, event)?;
    tx::commit(store, batch)?;
    Ok(seq)
}

/// Atomically replace the workflow + item projections and append the event.
pub fn commit(
    store: &rdb::store::Store,
    workflow: &TodoWorkflowRecord,
    items: &[TodoItemRecord],
    event: &TodoEventRecord,
) -> Result<i64> {
    let key = workflow_key(&workflow.id);
    let existing = get(store, &workflow.id)?
        .ok_or_else(|| anyhow::anyhow!("todo workflow generation conflict: {}", workflow.id))?;
    if existing.generation != workflow.generation - 1 {
        bail!("todo workflow generation conflict: {}", workflow.id);
    }
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &key, workflow)?;
    super::items::stage_upsert(&mut batch, items);
    let seq = super::events::stage_append(store, &mut batch, event)?;
    tx::commit(store, batch)?;
    Ok(seq)
}

/// Workflows by most-recent update, bounded by `limit`.
pub fn list(store: &rdb::store::Store, limit: u32) -> Result<Vec<TodoWorkflowSummary>> {
    let mut rows = Vec::new();
    for (_, value) in tx::scan_prefix(store, b"twf/")? {
        let w: TodoWorkflowRecord = crate::rdb_store::kv::decode(&value)?;
        rows.push(w);
    }
    rows.sort_by_key(|w| std::cmp::Reverse(w.updated_at));
    rows.truncate(limit as usize);
    Ok(rows
        .into_iter()
        .map(|w| TodoWorkflowSummary {
            id: w.id,
            status: w.status,
            parent_session_id: w.parent_session_id,
            generation: w.generation,
            updated_at: w.updated_at,
        })
        .collect())
}

/// Inspection projection with 64 KiB field omission markers.
pub fn get_detail(store: &rdb::store::Store, id: &str) -> Result<Option<TodoWorkflowDetail>> {
    let Some(w) = get(store, id)? else {
        return Ok(None);
    };
    let spec = serde_json::to_vec(&w.spec_json)?;
    let state = serde_json::to_vec(&w.state_json)?;
    Ok(Some(TodoWorkflowDetail {
        id: w.id,
        parent_session_id: w.parent_session_id,
        status: w.status,
        spec_json: bounded_json(&spec, "workflow.spec_json")?,
        state_json: bounded_json(&state, "workflow.state_json")?,
        generation: w.generation,
        created_at: w.created_at,
        updated_at: w.updated_at,
        terminal_reason: w.terminal_reason,
    }))
}

/// JSON field reader: values above the bound become an omission marker.
pub(crate) fn bounded_json(bytes: &[u8], field: &str) -> Result<serde_json::Value> {
    const BOUND: usize = 64 * 1024;
    if bytes.len() <= BOUND {
        return Ok(serde_json::from_slice(bytes)?);
    }
    Ok(serde_json::json!({
        "omitted": true,
        "total_bytes": bytes.len(),
        "read_via": "detail_field",
        "field": field,
    }))
}

/// Chunked read of a workflow field (SQL `substr` over the stored column).
pub fn field_chunk(
    store: &rdb::store::Store,
    id: &str,
    field: &str,
    offset: u64,
    max_bytes: usize,
) -> Result<Option<crate::PayloadChunkRecord>> {
    let Some(w) = get(store, id)? else {
        return Ok(None);
    };
    let bytes = match field {
        "spec_json" => serde_json::to_vec(&w.spec_json)?,
        "state_json" => serde_json::to_vec(&w.state_json)?,
        _ => bail!("unsupported todo workflow field"),
    };
    let total = bytes.len() as u64;
    if offset > total {
        bail!("todo workflow field offset exceeds total bytes");
    }
    let take = max_bytes.clamp(1, 64 * 1024);
    let start = offset as usize;
    let end = start.saturating_add(take).min(bytes.len());
    Ok(Some(crate::PayloadChunkRecord {
        total_bytes: total,
        bytes: bytes[start..end].to_vec(),
    }))
}

/// Cascade-delete the workflows of one session (`parent_session_id` FK).
///
/// Stages the workflow row deletion plus every item/event child; callers
/// commit the batch together with the session deletion.
pub fn delete_for_session(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    session_id: &str,
) -> Result<()> {
    for (key, value) in tx::scan_prefix(store, b"twf/")? {
        let workflow: TodoWorkflowRecord = crate::rdb_store::kv::decode(&value)?;
        if workflow.parent_session_id == session_id {
            batch.delete(&key);
            tx::delete_prefix(
                store,
                batch,
                &crate::rdb_store::kv::todo_item_prefix(&workflow.id),
            )?;
            tx::delete_prefix(
                store,
                batch,
                &crate::rdb_store::kv::todo_event_prefix(&workflow.id),
            )?;
        }
    }
    Ok(())
}

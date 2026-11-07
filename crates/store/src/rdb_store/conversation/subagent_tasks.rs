//! Subagent tasks (`sub/<task_id>` + parent/child indexes).
//!
//! Root rows are keyed by task id (point lookup); two indexes carry the
//! cascade deletes (both FK columns reference sessions) and the list
//! fence. The parent index value is the per-parent insertion ordinal,
//! which is what `list_subagent_tasks` sorts by.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::rdb_store::kv::{
    decode, subagent_child_index, subagent_child_prefix, subagent_key, subagent_parent_index,
    subagent_parent_prefix,
};
use crate::rdb_store::tx;
use crate::types::{SubagentStatus, SubagentTaskRecord};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentRow {
    #[serde(flatten)]
    pub record: SubagentTaskRecord,
    /// Per-parent insertion ordinal (list ordering).
    pub seq: i64,
}

pub fn create(store: &rdb::store::Store, rec: &SubagentTaskRecord) -> Result<()> {
    let seq = next_parent_ord(store, &rec.parent_session_id)?;
    let row = SubagentRow {
        record: rec.clone(),
        seq,
    };
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &subagent_key(&rec.task_id), &row)?;
    tx::put(
        &mut batch,
        &subagent_parent_index(&rec.parent_session_id, &rec.task_id),
        &seq,
    )?;
    tx::put_raw(
        &mut batch,
        &subagent_child_index(&rec.child_session_id, &rec.task_id),
        &[],
    );
    // Inventory change log (SQL trigger on subagent_tasks INSERT, keyed by
    // the child session id).
    crate::rdb_store::execution::inventory::stage(
        store,
        &mut batch,
        &[("session", &rec.child_session_id)],
    )?;
    tx::commit(store, batch)
}

fn next_parent_ord(store: &rdb::store::Store, parent: &str) -> Result<i64> {
    let mut max = 0i64;
    for (_, value) in tx::scan_prefix(store, &subagent_parent_prefix(parent))? {
        if let Ok(ord) = decode::<i64>(&value) {
            max = max.max(ord);
        }
    }
    Ok(max + 1)
}

/// Complete a running/cancelled task; terminal states reject the update.
pub fn complete(store: &rdb::store::Store, task_id: &str, result: &str, ok: bool) -> Result<()> {
    let status = if ok {
        SubagentStatus::Completed
    } else {
        SubagentStatus::Failed
    };
    let Some(mut row) = get_row(store, task_id)? else {
        bail!("subagent_task not found: {task_id}");
    };
    if !matches!(
        row.record.status,
        SubagentStatus::Running | SubagentStatus::Cancelled
    ) {
        bail!("subagent_task not found: {task_id}");
    }
    row.record.result = Some(result.to_string());
    row.record.ok = Some(ok);
    row.record.status = status;
    row.record.completed_at = Some(opencoder_core::message::now_ms());
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &subagent_key(task_id), &row)?;
    tx::commit(store, batch)
}

/// Mark a task cancelled (no result; the parent tool_use stays open).
pub fn cancel(store: &rdb::store::Store, task_id: &str) -> Result<()> {
    let Some(mut row) = get_row(store, task_id)? else {
        bail!("subagent_task not found: {task_id}");
    };
    row.record.status = SubagentStatus::Cancelled;
    row.record.completed_at = Some(opencoder_core::message::now_ms());
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &subagent_key(task_id), &row)?;
    tx::commit(store, batch)
}

/// All tasks of a parent session, insertion order.
pub fn list(store: &rdb::store::Store, parent_session_id: &str) -> Result<Vec<SubagentTaskRecord>> {
    let mut rows = Vec::new();
    for (key, value) in tx::scan_prefix(store, &subagent_parent_prefix(parent_session_id))? {
        let Some(task_id) = task_id_of(&key, &subagent_parent_prefix(parent_session_id)) else {
            continue;
        };
        if let Some(row) = get_row(store, &task_id)? {
            let ord = decode::<i64>(&value).unwrap_or(0);
            rows.push((ord, row));
        }
    }
    rows.sort_by_key(|(ord, _)| *ord);
    Ok(rows.into_iter().map(|(_, r)| r.record).collect())
}

pub fn get(store: &rdb::store::Store, task_id: &str) -> Result<Option<SubagentTaskRecord>> {
    Ok(get_row(store, task_id)?.map(|r| r.record))
}

fn get_row(store: &rdb::store::Store, task_id: &str) -> Result<Option<SubagentRow>> {
    crate::rdb_store::kv::get_json(store, &subagent_key(task_id))
}

fn task_id_of(key: &[u8], prefix: &[u8]) -> Option<String> {
    let suffix = key.strip_prefix(prefix)?;
    String::from_utf8(suffix.to_vec()).ok()
}

/// Cascade: remove every task whose parent OR child is `session_id`.
pub fn delete_for_session(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    session_id: &str,
) -> Result<()> {
    let mut task_ids = Vec::new();
    for (key, _) in tx::scan_prefix(store, &subagent_parent_prefix(session_id))? {
        if let Some(task_id) = task_id_of(&key, &subagent_parent_prefix(session_id)) {
            task_ids.push(task_id.clone());
            tx::del(batch, &key);
        }
    }
    for (key, _) in tx::scan_prefix(store, &subagent_child_prefix(session_id))? {
        if let Some(task_id) = task_id_of(&key, &subagent_child_prefix(session_id)) {
            task_ids.push(task_id.clone());
            tx::del(batch, &key);
        }
    }
    for task_id in task_ids {
        // Drop the row and both of its index entries.
        if let Some(row) = get_row(store, &task_id)? {
            // Inventory change log (SQL trigger on subagent_tasks DELETE,
            // keyed by the child session id).
            crate::rdb_store::execution::inventory::stage(
                store,
                batch,
                &[("session", &row.record.child_session_id)],
            )?;
            tx::del(
                batch,
                &subagent_parent_index(&row.record.parent_session_id, &task_id),
            );
            tx::del(
                batch,
                &subagent_child_index(&row.record.child_session_id, &task_id),
            );
        }
        tx::del(batch, &subagent_key(&task_id));
    }
    Ok(())
}

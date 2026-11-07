//! Worker-node registry (`nodes`) over the rdb KV engine.
//!
//! Key families:
//! - `node/<id>`  node root row
//! - `idx/node-name/<name>` → owning node id  (UNIQUE name)
//! - `ntask/<node_id>/<task_id>`  node-task queue rows — the builders live
//!   here because `heartbeat_node` (cancel-command poll) and the
//!   `delete_node` cascade must agree with the queue's layout; the task
//!   queue's own write paths (dispatch/claim/transition) arrive with the
//!   execution-ledger phase and reuse these keys.
//!
//! `register` upserts by name: a known name keeps its id (dispatched tasks
//! keep their foreign key) while liveness metadata refreshes; a new name
//! gets a fresh ULID.

use anyhow::{anyhow, bail, Result};

use crate::rdb_store::kv::{self};
use crate::rdb_store::tx;
use crate::rdb_store::{conversation, index, todo};
use crate::types::{NodeRecord, NodeTaskRecord, NodeTaskStatus};

pub fn node_key(id: &str) -> Vec<u8> {
    format!("node/{id}").into_bytes()
}

pub fn node_name_index(name: &str) -> Vec<u8> {
    format!("idx/node-name/{name}").into_bytes()
}

pub fn node_task_prefix(node_id: &str) -> Vec<u8> {
    format!("ntask/{node_id}/").into_bytes()
}

pub fn node_task_key(node_id: &str, task_id: &str) -> Vec<u8> {
    format!("ntask/{node_id}/{task_id}").into_bytes()
}

/// Register (or re-register) a node by its unique `name`; returns the stored
/// row either way.
pub fn register(
    store: &rdb::store::Store,
    name: &str,
    version: Option<&str>,
    workdir: Option<&str>,
    addr: Option<&str>,
    now_ms: i64,
) -> Result<NodeRecord> {
    let name_index = node_name_index(name);
    let mut batch = rocksdb::WriteBatch::default();
    let record = match index::owner_str(store, &name_index)? {
        // Known name: keep id and birth metadata, refresh the rest. The
        // upsert overwrites version/workdir/addr with the new values —
        // including `None`s — exactly like the SQL `ON CONFLICT` clause.
        // The index owner IS the node row key.
        Some(row_key) => {
            let mut row = kv::get_json::<NodeRecord>(store, row_key.as_bytes())?
                .ok_or_else(|| anyhow!("register_node: row for {name:?} vanished after upsert"))?;
            row.version = version.map(str::to_string);
            row.workdir = workdir.map(str::to_string);
            row.last_seen_at = now_ms;
            row.last_status = "online".to_string();
            row.last_addr = addr.map(str::to_string);
            row
        }
        None => {
            let record = NodeRecord {
                id: ulid::Ulid::new().to_string(),
                name: name.to_string(),
                version: version.map(str::to_string),
                workdir: workdir.map(str::to_string),
                first_seen: now_ms,
                last_seen_at: now_ms,
                last_status: "online".to_string(),
                last_task_id: None,
                last_addr: addr.map(str::to_string),
            };
            index::claim_unique(
                store,
                &mut batch,
                &name_index,
                &node_key(&record.id),
                "nodes.name",
            )?;
            record
        }
    };
    tx::put(&mut batch, &node_key(&record.id), &record)?;
    tx::commit(store, batch)?;
    Ok(record)
}

/// Every node, oldest registration first.
pub fn list(store: &rdb::store::Store) -> Result<Vec<NodeRecord>> {
    let mut rows = Vec::new();
    for (_, value) in tx::scan_prefix(store, b"node/")? {
        rows.push(kv::decode::<NodeRecord>(&value)?);
    }
    rows.sort_by(|a, b| (&a.first_seen, &a.id).cmp(&(&b.first_seen, &b.id)));
    Ok(rows)
}

pub fn get(store: &rdb::store::Store, id: &str) -> Result<Option<NodeRecord>> {
    kv::get_json(store, &node_key(id))
}

/// Delete a node; its task rows and — through them — each task's synthetic
/// session go in the same batch (the schema's double `ON DELETE CASCADE`).
pub fn delete(store: &rdb::store::Store, id: &str) -> Result<()> {
    let Some(row) = kv::get_json::<NodeRecord>(store, &node_key(id))? else {
        return Ok(());
    };
    let mut batch = rocksdb::WriteBatch::default();
    tx::del(&mut batch, &node_key(id));
    tx::del(&mut batch, &node_name_index(&row.name));
    for (key, value) in tx::scan_prefix(store, &node_task_prefix(id))? {
        if let Ok(task) = kv::decode::<NodeTaskRecord>(&value) {
            stage_session_delete(store, &mut batch, &task.session_id)?;
        }
        tx::del(&mut batch, &key);
    }
    tx::commit(store, batch)
}

/// Liveness touch + cancel-command poll in one write. Refreshes
/// `last_seen_at`, collapses non-`busy` status toward `idle`, and returns
/// the ids of this node's cancelling tasks (`created_at, id` order).
pub fn heartbeat(store: &rdb::store::Store, id: &str, now_ms: i64) -> Result<Vec<String>> {
    let key = node_key(id);
    let Some(mut row) = kv::get_json::<NodeRecord>(store, &key)? else {
        bail!("heartbeat_node: node {id} not found");
    };
    row.last_seen_at = now_ms;
    if row.last_status != "busy" {
        row.last_status = "idle".to_string();
    }
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &key, &row)?;
    tx::commit(store, batch)?;
    cancelling_tasks(store, id)
}

/// This node's cancel commands: tasks flagged `cancel_requested` while in
/// `cancelling`, oldest first.
fn cancelling_tasks(store: &rdb::store::Store, node_id: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for (_, value) in tx::scan_prefix(store, &node_task_prefix(node_id))? {
        let task: NodeTaskRecord = kv::decode(&value)?;
        if task.status == NodeTaskStatus::Cancelling && task.cancel_requested {
            out.push((task.created_at, task.id));
        }
    }
    out.sort();
    Ok(out.into_iter().map(|(_, id)| id).collect())
}

/// Stage a full synthetic-session delete (row + message/event/input
/// children + subagent and todo cascades) inside the caller's batch — the
/// same pieces `sessions::delete` commits on its own.
fn stage_session_delete(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    session_id: &str,
) -> Result<()> {
    batch.delete(kv::session_key(session_id));
    tx::delete_prefix(store, batch, &kv::message_prefix(session_id))?;
    tx::delete_prefix(store, batch, &kv::event_prefix(session_id))?;
    conversation::sessions::delete_input_children(store, batch, session_id)?;
    conversation::subagent_tasks::delete_for_session(store, batch, session_id)?;
    todo::workflows::delete_for_session(store, batch, session_id)?;
    Ok(())
}

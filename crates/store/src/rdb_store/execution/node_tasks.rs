//! Node task queue (`node_tasks`) over the rdb KV engine: dispatch, claim,
//! transitions, cancel polling, lost-node convergence, dialog clears.
//! Ported from the SQL backend's `execution/node_tasks` (free functions,
//! one atomic batch per operation under the engine's write lease).
//!
//! Key families (row prefix reused from `nodes` so its heartbeat poll and
//! delete cascade keep working unchanged):
//! - `ntask/<node_id>/<ord:020><task_id>`  task row (value = the record
//!   JSON, decodable by `nodes`' scans). The zero-padded GLOBAL insertion
//!   ordinal is the KV stand-in for SQL's `rowid`: it breaks same-millisecond
//!   `created_at` ties in exactly the dispatch order the SQL backend used.
//! - `idx/ntask-id/<task_id>` → row key      (PK lookup, self-healing)
//! - `idx/ntask-session/<session_id>` → row key (UNIQUE: one task/session)
//!
//! The unique indexes are validated against the LIVE row: an entry whose row
//! vanished (the node-delete cascade removes rows without knowing these
//! indexes) reads as free, so a stale entry never fakes a conflict.

use anyhow::{bail, Result};

use crate::rdb_store::conversation::{self};
use crate::rdb_store::execution::{node_state, nodes};
use crate::rdb_store::kv::{self, pad_ord};
use crate::rdb_store::todo;
use crate::rdb_store::{index, seq, tx};
use crate::types::{NodeTaskRecord, NodeTaskStatus, SessionMeta, TASK_TYPE_NODE};

include!("node_tasks/parts/part_1.rs");
include!("node_tasks/parts/part_2.rs");

/// Persisted global insertion counter family (the `rowid` stand-in).
const ROW_FAMILY: &str = "ntask-row";

pub(crate) fn task_id_index(task_id: &str) -> Vec<u8> {
    format!("idx/ntask-id/{task_id}").into_bytes()
}

pub(crate) fn session_index(session_id: &str) -> Vec<u8> {
    format!("idx/ntask-session/{session_id}").into_bytes()
}

fn task_row_key(node_id: &str, row_ord: i64, task_id: &str) -> Vec<u8> {
    let mut k = nodes::node_task_prefix(node_id);
    k.extend_from_slice(pad_ord(row_ord).as_bytes());
    k.extend_from_slice(task_id.as_bytes());
    k
}

/// Split `ntask/<node>/<ord:020><task_id>` into its three parts.
pub(crate) fn parse_task_key(key: &[u8]) -> Option<(String, i64, String)> {
    let text = std::str::from_utf8(key).ok()?;
    let rest = text.strip_prefix("ntask/")?;
    let (node_id, leaf) = rest.split_once('/')?;
    if leaf.len() <= kv::ORD_WIDTH {
        return None;
    }
    let (ord, task_id) = leaf.split_at(kv::ORD_WIDTH);
    Some((node_id.to_string(), ord.parse().ok()?, task_id.to_string()))
}

/// Owner recorded for a unique index, but only when the row is still alive;
/// dangling entries (see the module doc) read as free.
fn live_owner(store: &rdb::store::Store, index_key: &[u8]) -> Result<Option<Vec<u8>>> {
    match index::owner(store, index_key)? {
        Some(row_key) => {
            if rdb::store::get(store, b"", &row_key)
                .map_err(kv::engine_err)?
                .is_some()
            {
                Ok(Some(row_key))
            } else {
                Ok(None)
            }
        }
        None => Ok(None),
    }
}

/// Reserve both unique indexes for `row_key` in `batch`, mapping a conflict
/// to the SQL `UNIQUE constraint failed:` text.
fn claim_indexes(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    row_key: &[u8],
    task_id: &str,
    session_id: &str,
) -> Result<()> {
    let id_index = task_id_index(task_id);
    if let Some(existing) = live_owner(store, &id_index)? {
        if existing.as_slice() != row_key {
            bail!("UNIQUE constraint failed: node_tasks.id");
        }
    }
    let ses_index = session_index(session_id);
    if let Some(existing) = live_owner(store, &ses_index)? {
        if existing.as_slice() != row_key {
            bail!("UNIQUE constraint failed: node_tasks.session_id");
        }
    }
    batch.put(&id_index, row_key);
    batch.put(&ses_index, row_key);
    Ok(())
}

/// Fresh-task fields, grouped so the staging helper keeps a small signature.
struct NewNodeTask<'a> {
    task_id: &'a str,
    session_id: &'a str,
    node_id: &'a str,
    title: Option<&'a str>,
    prompt: &'a str,
    agent: Option<&'a str>,
    model: Option<&'a str>,
    now_ms: i64,
}

/// Build the fresh `pending` task row staged with its indexes.
fn stage_new_task(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    spec: &NewNodeTask<'_>,
) -> Result<NodeTaskRecord> {
    let record = NodeTaskRecord {
        id: spec.task_id.to_string(),
        node_id: spec.node_id.to_string(),
        session_id: spec.session_id.to_string(),
        title: spec.title.map(str::to_string),
        prompt: spec.prompt.to_string(),
        agent: spec.agent.map(str::to_string),
        model: spec.model.map(str::to_string),
        status: NodeTaskStatus::Pending,
        error: None,
        cancel_requested: false,
        created_at: spec.now_ms,
        claimed_at: None,
        finished_at: None,
    };
    let row_ord = seq::next_global(store, batch, ROW_FAMILY)?;
    let row_key = task_row_key(spec.node_id, row_ord, spec.task_id);
    claim_indexes(store, batch, &row_key, spec.task_id, spec.session_id)?;
    tx::put(batch, &row_key, &record)?;
    Ok(record)
}

/// Enqueue a node task plus its synthetic session (`task_type == "node"`)
/// in ONE batch. Errors when the node does not exist.
#[allow(clippy::too_many_arguments)]
pub fn dispatch(
    store: &rdb::store::Store,
    task_id: &str,
    session_id: &str,
    node_id: &str,
    title: Option<&str>,
    prompt: &str,
    agent: Option<&str>,
    model: Option<&str>,
    now_ms: i64,
) -> Result<NodeTaskRecord> {
    if nodes::get(store, node_id)?.is_none() {
        bail!("dispatch_node_task: node {node_id} does not exist");
    }
    let mut batch = rocksdb::WriteBatch::default();
    // Session first (INSERT OR IGNORE semantics): node_tasks.session_id is an
    // immediate FK into it.
    let session_key = kv::session_key(session_id);
    if rdb::store::get(store, b"", &session_key)
        .map_err(kv::engine_err)?
        .is_none()
    {
        let meta = synthetic_session(session_id, title, agent, model, now_ms);
        tx::put(
            &mut batch,
            &session_key,
            &conversation::sessions::SessionRow::from(meta),
        )?;
    }
    let record = stage_new_task(
        store,
        &mut batch,
        &NewNodeTask {
            task_id,
            session_id,
            node_id,
            title,
            prompt,
            agent,
            model,
            now_ms,
        },
    )?;
    tx::commit(store, batch)?;
    Ok(record)
}

/// Enqueue a node task bound to an EXISTING session (dialog resume): the
/// session row is reused untouched — only the task row appears.
#[allow(clippy::too_many_arguments)]
pub fn dispatch_for_session(
    store: &rdb::store::Store,
    task_id: &str,
    session_id: &str,
    node_id: &str,
    title: Option<&str>,
    prompt: &str,
    agent: Option<&str>,
    model: Option<&str>,
    now_ms: i64,
) -> Result<NodeTaskRecord> {
    if nodes::get(store, node_id)?.is_none() {
        bail!("dispatch_node_task: node {node_id} does not exist");
    }
    if conversation::sessions::get(store, session_id)?.is_none() {
        bail!("dispatch_node_task: session {session_id} does not exist");
    }
    let mut batch = rocksdb::WriteBatch::default();
    let record = stage_new_task(
        store,
        &mut batch,
        &NewNodeTask {
            task_id,
            session_id,
            node_id,
            title,
            prompt,
            agent,
            model,
            now_ms,
        },
    )?;
    tx::commit(store, batch)?;
    Ok(record)
}

fn synthetic_session(
    session_id: &str,
    title: Option<&str>,
    agent: Option<&str>,
    model: Option<&str>,
    now_ms: i64,
) -> SessionMeta {
    SessionMeta {
        kind: None,
        id: session_id.to_string(),
        title: title.map(str::to_string),
        agent: agent.map(str::to_string),
        model: model.map(str::to_string),
        autopilot_mode: None,
        workdir_hash: None,
        created_at: now_ms,
        updated_at: now_ms,
        summary: None,
        summary_seq: None,
        summary_images: Vec::new(),
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: Some(TASK_TYPE_NODE.to_string()),
        requirement: None,
    }
}

/// Atomically claim the oldest pending task of `node_id` (FIFO by
/// `created_at`, insertion ordinal on ties). `None` when the node already
/// runs a task (single-active policy) or nothing is queued. The scan and the
/// conditional write run under the engine's exclusive write lease inside one
/// batch, so racing claimers serialize exactly like `BEGIN IMMEDIATE` + the
/// SQL `UPDATE ... WHERE status='pending'` CAS.
pub fn claim_next(
    store: &rdb::store::Store,
    node_id: &str,
    now_ms: i64,
) -> Result<Option<NodeTaskRecord>> {
    let mut busy = false;
    let mut pending: Vec<(i64, NodeTaskRecord)> = Vec::new();
    for (key, value) in tx::scan_prefix(store, &nodes::node_task_prefix(node_id))? {
        let Some((_, row_ord, _)) = parse_task_key(&key) else {
            continue;
        };
        let task: NodeTaskRecord = kv::decode(&value)?;
        match task.status {
            NodeTaskStatus::Running | NodeTaskStatus::Cancelling => busy = true,
            NodeTaskStatus::Pending => pending.push((row_ord, task)),
            _ => {}
        }
    }
    if busy {
        return Ok(None);
    }
    pending.sort_by(|a, b| (&a.1.created_at, &a.0).cmp(&(&b.1.created_at, &b.0)));
    let Some((row_ord, mut task)) = pending.into_iter().next() else {
        return Ok(None);
    };
    task.status = NodeTaskStatus::Running;
    task.claimed_at = Some(now_ms);
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &task_row_key(node_id, row_ord, &task.id), &task)?;
    stage_mark_busy(store, &mut batch, node_id, &task.id)?;
    tx::commit(store, batch)?;
    Ok(Some(task))
}

/// Mark the node busy on the claimed task (0-row SQL UPDATE tolerated).
fn stage_mark_busy(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    node_id: &str,
    task_id: &str,
) -> Result<()> {
    if let Some(mut node) = nodes::get(store, node_id)? {
        node.last_status = "busy".to_string();
        node.last_task_id = Some(task_id.to_string());
        tx::put(batch, &nodes::node_key(node_id), &node)?;
    }
    Ok(())
}

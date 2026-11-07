//! Node registry, five-column execution index queries and definitions KV.
use super::keys::{self, ord_key_id, XALL_PREFIX};
use super::FleetStore;
use crate::rdb_store::kv;
use crate::rdb_store::tx;
use anyhow::{bail, Result};
use opencoder_core::fleet::{
    ExecutionCursor, ExecutionIndex, ExecutionKind, ExecutionPage, NodeRegistration,
    EXECUTION_PAGE_MAX,
};
use serde_json::Value;

const TERMINAL_DELETABLE: [&str; 4] = ["idle", "done", "error", "cancelled"];

pub(super) fn register(store: &rdb::store::Store, node: &NodeRegistration) -> Result<()> {
    let mut batch = rocksdb::WriteBatch::default();
    batch.put(keys::node_key(&node.id), kv::encode(node)?);
    tx::commit(store, batch)
}

/// Remove only the node registration; execution ownership and history
/// remain in the execution index and in the node's own durable store.
pub(super) fn unregister(store: &rdb::store::Store, id: &str) -> Result<bool> {
    let mut batch = rocksdb::WriteBatch::default();
    let key = keys::node_key(id);
    let removed = rdb::store::get(store, b"", &key)
        .map_err(kv::engine_err)?
        .is_some();
    if removed {
        batch.delete(&key);
        tx::commit(store, batch)?;
    }
    Ok(removed)
}

pub(super) fn nodes(store: &rdb::store::Store) -> Result<Vec<NodeRegistration>> {
    let mut result = Vec::new();
    for (_, value) in tx::scan_prefix(store, &keys::node_prefix())? {
        result.push(kv::decode(&value)?);
    }
    Ok(result) // key order = id order
}

/// Ownership and creation time are immutable, including after reconnect.
pub(super) fn put_index(store: &rdb::store::Store, record: &ExecutionIndex) -> Result<()> {
    let existing = keys::read_index(store, &record.id)?;
    if let Some(old) = &existing {
        if old.node_id != record.node_id
            || old.created_at != record.created_at
            || old.kind != record.kind
        {
            bail!("execution ownership conflict: {}", record.id);
        }
    }
    let mut batch = rocksdb::WriteBatch::default();
    if existing.is_some() {
        // Order keys derive from immutable fields, so only the row changes.
        batch.put(keys::xrow_key(&record.id), kv::encode(record)?);
    } else {
        keys::put_index_row(&mut batch, record)?;
    }
    tx::commit(store, batch)
}

/// Remove execution-index rows by id, ownership-guarded by `node_id` and
/// `kind`, together with their frozen dispatch assignments and execution
/// receipts (no foreign keys link these rows, so all three go in one
/// batch). Rows whose stored status is NOT terminal are refused so a stale
/// id list can never drop a live execution. Returns the number of deleted
/// index rows.
pub(super) fn delete_terminal_indexes(
    store: &rdb::store::Store,
    node_id: &str,
    kind: ExecutionKind,
    ids: &[String],
) -> Result<u64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let mut batch = rocksdb::WriteBatch::default();
    let mut removed = 0u64;
    for id in ids {
        let Some(record) = keys::read_index(store, id)? else {
            continue;
        };
        if record.node_id != node_id
            || record.kind != kind
            || !TERMINAL_DELETABLE.contains(&record.status.as_str())
        {
            continue;
        }
        removed += 1;
        delete_index_entry(&mut batch, &record);
        batch.delete(keys::asg_key(id));
        if record.kind == ExecutionKind::Project {
            delete_project_run_index(store, &mut batch, id)?;
        }
        batch.delete(keys::rcp_key("execution", id));
    }
    tx::commit(store, batch)?;
    Ok(removed)
}

pub(super) fn delete_index_entry(batch: &mut rocksdb::WriteBatch, record: &ExecutionIndex) {
    batch.delete(keys::xrow_key(&record.id));
    batch.delete(keys::xall_key(record.created_at, &record.id));
    batch.delete(keys::xnk_key(
        &record.node_id,
        record.kind.prefix(),
        record.created_at,
        &record.id,
    ));
}

/// The project run-key index points at the assignment's dispatch key; the
/// frozen assignment is the only place it is recorded.
fn delete_project_run_index(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    id: &str,
) -> Result<()> {
    if let Some(raw) = rdb::store::get(store, b"", &keys::asg_key(id)).map_err(kv::engine_err)? {
        let assignment: opencoder_core::fleet::Assignment = kv::decode(&raw)?;
        if assignment.request.kind == ExecutionKind::Project {
            batch.delete(keys::prk_key(super::handoff::dispatch_key(
                &assignment.request,
            )));
        }
    }
    Ok(())
}

pub(super) fn index(store: &rdb::store::Store, id: &str) -> Result<Option<ExecutionIndex>> {
    keys::read_index(store, id)
}

fn matches(record: &ExecutionIndex, node: Option<&str>, kind: Option<ExecutionKind>) -> bool {
    node.is_none_or(|node| record.node_id == node) && kind.is_none_or(|kind| record.kind == kind)
}

pub(super) fn indexes(
    store: &rdb::store::Store,
    node: Option<&str>,
    kind: Option<ExecutionKind>,
    limit: u32,
) -> Result<Vec<ExecutionIndex>> {
    let limit = limit.clamp(1, 10000) as usize;
    collect_page(store, node, kind, None, limit)
}

pub(super) fn indexes_page(
    store: &rdb::store::Store,
    node: Option<&str>,
    kind: Option<ExecutionKind>,
    cursor: Option<&ExecutionCursor>,
    limit: u32,
) -> Result<ExecutionPage<ExecutionIndex>> {
    let limit = limit.clamp(1, EXECUTION_PAGE_MAX) as usize;
    let mut executions = collect_page(store, node, kind, cursor, limit + 1)?;
    let more = executions.len() > limit;
    executions.truncate(limit);
    let next_cursor = more.then(|| {
        let last = executions.last().expect("non-empty page with extra row");
        ExecutionCursor {
            created_at: last.created_at,
            id: last.id.clone(),
        }
    });
    Ok(ExecutionPage {
        executions,
        next_cursor,
    })
}

/// `ORDER BY created_at DESC, id ASC` with the `(node, kind)` projection when
/// both filters are set (mirrors the SQL composite lookup index); otherwise
/// the global order stream is filtered in memory, like the SQL table scan.
/// Returns at most `take` rows.
fn collect_page(
    store: &rdb::store::Store,
    node: Option<&str>,
    kind: Option<ExecutionKind>,
    cursor: Option<&ExecutionCursor>,
    take: usize,
) -> Result<Vec<ExecutionIndex>> {
    let mut executions: Vec<ExecutionIndex> = Vec::new();
    let full = take;
    let mut visit = |record: Option<ExecutionIndex>| -> bool {
        if let Some(record) = record {
            if matches(&record, node, kind) {
                executions.push(record);
            }
        }
        executions.len() < full
    };
    if let (Some(node), Some(kind)) = (node, kind) {
        let prefix = keys::xnk_prefix(node, kind.prefix());
        let from = match cursor {
            Some(cursor) => {
                let mut key = prefix.clone();
                key.extend_from_slice(
                    &keys::xall_key(cursor.created_at, &cursor.id)[XALL_PREFIX.len()..],
                );
                key.push(0);
                key
            }
            None => prefix.clone(),
        };
        scan_order(store, &prefix, &from, &mut visit)?;
    } else {
        let from = cursor
            .map(|cursor| keys::after_xall(cursor.created_at, &cursor.id))
            .unwrap_or_else(|| XALL_PREFIX.to_vec());
        scan_order(store, XALL_PREFIX, &from, &mut visit)?;
    }
    Ok(executions)
}

/// Walk an order-index family ascending from `from`, resolving each id to its
/// row; the callback decides when enough rows were collected.
fn scan_order(
    store: &rdb::store::Store,
    prefix: &[u8],
    from: &[u8],
    visit: &mut dyn FnMut(Option<ExecutionIndex>) -> bool,
) -> Result<()> {
    let mut error: Option<anyhow::Error> = None;
    rdb::store::for_each_from(store, from, false, &mut |key, _| {
        if !key.starts_with(prefix) {
            return false;
        }
        let Some(id) = ord_key_id(prefix, key) else {
            return true;
        };
        let record = match keys::read_index(store, &id) {
            Ok(record) => record,
            Err(err) => {
                error = Some(err);
                return false;
            }
        };
        visit(record)
    })
    .map_err(kv::engine_err)?;
    if let Some(error) = error {
        return Err(error);
    }
    Ok(())
}

pub(super) fn definition(store: &rdb::store::Store, kind: &str, id: &str) -> Result<Option<Value>> {
    match rdb::store::get(store, b"", &keys::def_key(kind, id)).map_err(kv::engine_err)? {
        Some(raw) => Ok(Some(super::inventory_meta::decode(
            kind,
            std::str::from_utf8(&raw)?,
        )?)),
        None => Ok(None),
    }
}

pub(super) fn definitions(store: &rdb::store::Store, kind: &str) -> Result<Vec<Value>> {
    let mut result = Vec::new();
    for (_, value) in tx::scan_prefix(store, &keys::def_prefix(kind))? {
        result.push(super::inventory_meta::decode(
            kind,
            std::str::from_utf8(&value)?,
        )?);
    }
    Ok(result) // key order = id order
}

pub(super) fn delete_definition(store: &rdb::store::Store, kind: &str, id: &str) -> Result<()> {
    let mut batch = rocksdb::WriteBatch::default();
    batch.delete(keys::def_key(kind, id));
    tx::commit(store, batch)
}

pub(super) fn put_definition(
    store: &rdb::store::Store,
    kind: &str,
    id: &str,
    body: &Value,
) -> Result<()> {
    let mut batch = rocksdb::WriteBatch::default();
    put_definition_batch(store, kind, id, body, &mut batch)?;
    tx::commit(store, batch)
}

pub(super) fn put_definition_batch(
    _store: &rdb::store::Store,
    kind: &str,
    id: &str,
    body: &Value,
    batch: &mut rocksdb::WriteBatch,
) -> Result<()> {
    batch.put(
        keys::def_key(kind, id),
        super::inventory_meta::encode(kind, body)?,
    );
    Ok(())
}

#[path = "access/records.rs"]
mod database_access;

// Publish a node's durable acceptance without replacing newer report status.
use super::super::keys;
use super::super::records;
use super::super::FleetStore;
use crate::rdb_store::{kv, tx};
use anyhow::{ensure, Result};
use opencoder_core::fleet::{Assignment, ExecutionIndex};

/// Validate a pre-existing index row against an accepted index (same
/// immutable owner fields). Shared by prepare and acceptance paths.
pub(super) fn validate_row(store: &rdb::store::Store, accepted: &ExecutionIndex) -> Result<()> {
    if let Some(current) = records::index(store, &accepted.id)? {
        validate_owner(&current, accepted)?;
    }
    Ok(())
}

/// Stage the ownership checks and the insert-if-absent index row into
/// `batch` (callers commit it with their own writes in one atomic step).
pub(super) fn stage_insert(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    accepted: &ExecutionIndex,
    receipt_key: Option<&str>,
) -> Result<()> {
    if let Some(raw) =
        rdb::store::get(store, b"", &keys::asg_key(&accepted.id)).map_err(kv::engine_err)?
    {
        let assignment: Assignment = serde_json::from_slice(&raw)?;
        validate_owner(&assignment.index, accepted)?;
        if let Some(key) = receipt_key {
            ensure!(
                super::receipts::dispatch_key(&assignment.request) == key,
                "acceptance does not match dispatch receipt"
            );
        }
    } else {
        ensure!(receipt_key.is_none(), "accepted assignment missing");
    }
    if records::index(store, &accepted.id)?.is_some() {
        // ON CONFLICT(id) DO NOTHING: keep the reported row's status.
        return Ok(());
    }
    keys::put_index_row(batch, accepted)
}

fn validate_owner(expected: &ExecutionIndex, actual: &ExecutionIndex) -> Result<()> {
    ensure!(
        expected.id == actual.id
            && expected.kind == actual.kind
            && expected.node_id == actual.node_id
            && expected.created_at == actual.created_at,
        "execution ownership conflict: {}",
        actual.id
    );
    Ok(())
}

pub(super) fn accept_execution(store: &rdb::store::Store, accepted: &ExecutionIndex) -> Result<()> {
    let mut batch = rocksdb::WriteBatch::default();
    stage_insert(store, &mut batch, accepted, None)?;
    tx::commit(store, batch)
}

#[path = "access/acceptance.rs"]
mod database_access;

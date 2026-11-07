//! Validate a Runtime inventory without querying ownership once per history row.
use super::super::keys;
use super::FleetStore;
use crate::rdb_store::{kv, tx};
use anyhow::{ensure, Result};

/// Existing ownership is immutable and stays read-only. Newly discovered
/// children are assigned atomically after rechecking under the writer lease.
/// Ids owned by a different runtime are an explicit conflict, not a skip.
pub(super) fn assign_runtime_inventory(
    store: &rdb::store::Store,
    runtime: &str,
    ids: &[&str],
) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    // Deduplicate like SQL's SELECT DISTINCT before the ownership scan.
    let mut unique = ids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    let mut unassigned = Vec::new();
    let mut missing = false;
    for id in &unique {
        match kv::get_json::<String>(store, &keys::own_key(id))? {
            Some(owner) => ensure!(owner == runtime, "execution runtime ownership conflict"),
            None => {
                missing = true;
                unassigned.push(id);
            }
        }
    }
    if !missing {
        return Ok(());
    }
    let mut batch = rocksdb::WriteBatch::default();
    ensure!(
        kv::get_json::<super::RuntimeRecord>(store, &keys::rt_key(runtime))?.is_some(),
        "no registered runtime"
    );
    for id in unassigned {
        tx::put(&mut batch, &keys::own_key(id), &runtime)?;
    }
    tx::commit(store, batch)
}

#[path = "access/inventory.rs"]
mod database_access;

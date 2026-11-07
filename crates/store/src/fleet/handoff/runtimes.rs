//! Host runtimes and immutable execution ownership.
use super::super::keys;
use super::FleetStore;
use crate::rdb_store::{kv, tx};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeRecord {
    pub id: String,
    pub release_id: String,
    pub config: Value,
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeOwner {
    pub execution_id: String,
    pub runtime_id: String,
    pub release_id: String,
}

pub(super) fn register_runtime(store: &rdb::store::Store, runtime: &RuntimeRecord) -> Result<()> {
    ensure!(
        opencoder_core::fleet::valid_id(&runtime.id),
        "invalid runtime id"
    );
    ensure!(
        opencoder_core::fleet::valid_id(&runtime.release_id),
        "invalid release id"
    );
    ensure!(runtime.mode == "staged", "runtime must register staged");
    let mut batch = rocksdb::WriteBatch::default();
    if kv::get_json::<RuntimeRecord>(store, &keys::rt_key(&runtime.id))?.is_none() {
        batch.put(keys::rt_key(&runtime.id), kv::encode(runtime)?);
        crate::rdb_store::tx::commit(store, batch)?;
    }
    let existing = kv::get_json::<RuntimeRecord>(store, &keys::rt_key(&runtime.id))?.unwrap();
    ensure!(
        existing.release_id == runtime.release_id && existing.config == runtime.config,
        "runtime identity already registered with different configuration"
    );
    Ok(())
}

pub(super) fn runtimes(store: &rdb::store::Store) -> Result<Vec<RuntimeRecord>> {
    let mut records = Vec::new();
    for (_, value) in crate::rdb_store::tx::scan_prefix(store, b"rt/")? {
        records.push(kv::decode(&value)?);
    }
    Ok(records) // key order = id order
}

pub(super) fn owner(store: &rdb::store::Store, execution_id: &str) -> Result<Option<RuntimeOwner>> {
    let Some(runtime_id) = kv::get_json::<String>(store, &keys::own_key(execution_id))? else {
        return Ok(None);
    };
    let runtime = kv::get_json::<RuntimeRecord>(store, &keys::rt_key(&runtime_id))?
        .context("runtime owner references a missing runtime")?;
    Ok(Some(RuntimeOwner {
        execution_id: execution_id.to_owned(),
        runtime_id,
        release_id: runtime.release_id,
    }))
}

/// Activation affects only previously unseen IDs. Ownership never moves.
/// The `rta/` singleton is the UNIQUE-active marker: retiring the previous
/// runtime and activating the new one commit in one batch.
pub(super) fn activate_runtime(store: &rdb::store::Store, id: &str) -> Result<()> {
    let mut batch = rocksdb::WriteBatch::default();
    let mut found = false;
    for (key, value) in crate::rdb_store::tx::scan_prefix(store, b"rt/")? {
        let mut record: RuntimeRecord = kv::decode(&value)?;
        if record.id == id {
            found = true;
            record.mode = "active".into();
        } else if record.mode == "active" {
            record.mode = "retired".into();
        } else {
            continue;
        }
        batch.put(key, kv::encode(&record)?);
    }
    ensure!(found, "candidate runtime is not registered");
    batch.put(keys::rt_active_key(), id.as_bytes());
    crate::rdb_store::tx::commit(store, batch)
}

/// First acceptance and activation serialize on the write lease. Children
/// explicitly inherit the reporting runtime; an existing row is never
/// overwritten.
pub(super) fn assign_runtime(
    store: &rdb::store::Store,
    execution_id: &str,
    inherited: Option<&str>,
) -> Result<String> {
    if let Some(owner) = owner(store, execution_id)? {
        ensure!(
            inherited.is_none_or(|id| id == owner.runtime_id),
            "execution runtime ownership conflict"
        );
        return Ok(owner.runtime_id);
    }
    let runtime = match inherited {
        Some(id) => {
            kv::get_json::<RuntimeRecord>(store, &keys::rt_key(id))?.context("no active runtime")?
        }
        None => {
            let active = rdb::store::get(store, b"", &keys::rt_active_key())
                .map_err(kv::engine_err)?
                .context("no active runtime")?;
            let id = String::from_utf8(active)?;
            kv::get_json::<RuntimeRecord>(store, &keys::rt_key(&id))?
                .context("no active runtime")?
        }
    };
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &keys::own_key(execution_id), &runtime.id)?;
    crate::rdb_store::tx::commit(store, batch)?;
    Ok(runtime.id)
}

#[path = "access/runtimes.rs"]
mod database_access;

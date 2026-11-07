//! Immutable brain plan versions plus the plan definition metadata row.
use super::keys;
use super::records;
use super::FleetStore;
use crate::rdb_store::kv;
use crate::rdb_store::tx;
use anyhow::{ensure, Result};
use opencoder_core::brain::{PlanDefinition, PlanVersion};
use opencoder_core::fleet::valid_id;
use serde_json::Value;

const KIND: &str = "brain_plan";
const HISTORY_LIMIT: usize = 20;

/// Remove a saved plan and every immutable version in one batch. Plan
/// capability metadata is derived from the saved versions, so it is removed
/// with the plan to avoid orphaned catalog entries. Returns `false` when the
/// plan metadata does not exist.
pub(super) fn delete_brain_plan(store: &rdb::store::Store, id: &str) -> Result<bool> {
    ensure!(valid_id(id), "invalid plan identity");
    if records::definition(store, KIND, id)?.is_none() {
        return Ok(false);
    }
    let mut batch = rocksdb::WriteBatch::default();
    batch.delete(keys::def_key(KIND, id));
    // Range tombstones staged in the same batch clear every version and
    // every capability entry atomically (`plan-<id>@` prefix rows included).
    super::keys::stage_delete_prefix(&mut batch, &keys::plan_prefix(id));
    let capability_prefix = [
        keys::def_prefix("brain_capability_meta"),
        format!("plan-{id}@").into_bytes(),
    ]
    .concat();
    super::keys::stage_delete_prefix(&mut batch, &capability_prefix);
    tx::commit(store, batch)?;
    Ok(true)
}

/// Append-only, compare-and-swap version publication. Retrying the same
/// exact version is idempotent; an existing version is never overwritten.
pub(super) fn save_brain_plan_document(
    store: &rdb::store::Store,
    version: &PlanVersion<Value>,
) -> Result<PlanDefinition> {
    ensure!(
        valid_id(&version.id) && version.version > 0 && version.version <= i64::MAX as u64,
        "invalid plan identity"
    );
    ensure!(
        !version.changelog.trim().is_empty(),
        "changelog is required"
    );
    let previous: Option<PlanDefinition> = records::definition(store, KIND, &version.id)?
        .map(serde_json::from_value)
        .transpose()?;
    if let Some(existing) = plan_document(store, &version.id, version.version)? {
        ensure!(existing == *version, "immutable plan version conflict");
        return previous.ok_or_else(|| anyhow::anyhow!("plan metadata missing"));
    }
    ensure!(
        version.version == previous.as_ref().map_or(1, |p| p.latest_version + 1),
        "plan version conflict; reload latest version"
    );
    let definition = PlanDefinition {
        id: version.id.clone(),
        title: version.plan["title"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("plan title missing"))?
            .into(),
        latest_version: version.version,
        stable_version: previous.and_then(|p| p.stable_version),
        updated_at: version.created_at,
    };
    let mut batch = rocksdb::WriteBatch::default();
    batch.put(
        keys::plan_key(&version.id, version.version),
        kv::encode(version)?,
    );
    records::put_definition_batch(
        store,
        KIND,
        &version.id,
        &serde_json::to_value(&definition)?,
        &mut batch,
    )?;
    tx::commit(store, batch)?;
    Ok(definition)
}

pub(super) fn plan_document(
    store: &rdb::store::Store,
    id: &str,
    version: u64,
) -> Result<Option<PlanVersion<Value>>> {
    kv::get_json(store, &keys::plan_key(id, version))
}

/// Newest versions first (`version DESC`), optionally strictly `before`.
pub(super) fn plan_documents(
    store: &rdb::store::Store,
    id: &str,
    before: Option<u64>,
) -> Result<Vec<PlanVersion<Value>>> {
    let prefix = keys::plan_prefix(id);
    let mut result = Vec::new();
    let mut error = None;
    tx::scan_down(store, &kv::prefix_upper(&prefix), false, |key, value| {
        if !key.starts_with(&prefix) {
            return false;
        }
        let document: PlanVersion<Value> = match kv::decode(value) {
            Ok(document) => document,
            Err(err) => {
                error = Some(err);
                return false;
            }
        };
        if before.is_some_and(|bound| document.version >= bound) {
            return true; // newer than the cursor: keep descending
        }
        result.push(document);
        result.len() < HISTORY_LIMIT
    })?;
    if let Some(error) = error {
        return Err(error);
    }
    Ok(result)
}

pub(super) fn mark_brain_stable(
    store: &rdb::store::Store,
    id: &str,
    version: u64,
) -> Result<PlanDefinition> {
    ensure!(
        plan_document(store, id, version)?.is_some(),
        "unknown plan version"
    );
    let mut definition: PlanDefinition = serde_json::from_value(
        records::definition(store, KIND, id)?.ok_or_else(|| anyhow::anyhow!("unknown plan"))?,
    )?;
    definition.stable_version = Some(version);
    let mut batch = rocksdb::WriteBatch::default();
    records::put_definition_batch(
        store,
        KIND,
        id,
        &serde_json::to_value(&definition)?,
        &mut batch,
    )?;
    tx::commit(store, batch)?;
    Ok(definition)
}

#[path = "access/brain.rs"]
mod database_access;

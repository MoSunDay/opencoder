//! Scalar ownership checks never parse frozen request bodies after warmup.
use super::super::keys;
use super::super::ownership;
use crate::rdb_store::kv;
use anyhow::{bail, Result};
use opencoder_core::fleet::ExecutionIndex;

/// Check every immutable field before changing any status, then upsert the
/// rows. The caller owns the engine's write lease, so another Server cannot
/// alter ownership between these steps. Unchanged rows are skipped so a
/// replayed inventory does not dirty every persisted row.
pub(super) fn apply(
    store: &rdb::store::Store,
    records: &[ExecutionIndex],
    batch: &mut rocksdb::WriteBatch,
) -> Result<()> {
    validate(store, records)?;
    for record in records {
        let existing = keys::read_index(store, &record.id)?;
        match existing {
            Some(old) => {
                if old.created_at != record.created_at
                    || old.kind != record.kind
                    || old.node_id != record.node_id
                {
                    bail!("execution ownership conflict: {}", record.id);
                }
                if old.status != record.status {
                    // Order keys derive from immutable fields: row-only write.
                    batch.put(keys::xrow_key(&record.id), kv::encode(record)?);
                }
            }
            None => keys::put_index_row(batch, record)?,
        }
    }
    Ok(())
}

pub(super) fn validate(store: &rdb::store::Store, records: &[ExecutionIndex]) -> Result<()> {
    for record in records {
        if let Some(old) = keys::read_index(store, &record.id)? {
            if old.created_at != record.created_at
                || old.kind != record.kind
                || old.node_id != record.node_id
            {
                bail!("execution ownership conflict: {}", record.id);
            }
        }
    }
    ownership::validate_unpublished(store, records)
}

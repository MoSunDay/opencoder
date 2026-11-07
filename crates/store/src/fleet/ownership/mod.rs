//! Published owners use the immutable execution index. Only first reports need
//! the frozen assignment; native preparation cannot change an indexed owner.
use super::keys;
use crate::rdb_store::kv;
use anyhow::{ensure, Result};
use opencoder_core::fleet::{Assignment, ExecutionIndex};

/// For incoming records whose id has a frozen assignment but NO index row,
/// the frozen owner must agree with the report. After first acceptance the
/// scalar index is authoritative, including while older Servers still write.
pub(super) fn validate_unpublished(
    store: &rdb::store::Store,
    records: &[ExecutionIndex],
) -> Result<()> {
    for record in records {
        if keys::read_index(store, &record.id)?.is_some() {
            continue;
        }
        let Some(raw) =
            rdb::store::get(store, b"", &keys::asg_key(&record.id)).map_err(kv::engine_err)?
        else {
            continue;
        };
        let owner: Assignment = kv::decode(&raw)?;
        ensure!(
            owner.index.id == record.id
                && owner.index.created_at == record.created_at
                && owner.index.kind == record.kind
                && owner.index.node_id == record.node_id,
            "execution ownership conflict: {}",
            record.id
        );
    }
    Ok(())
}

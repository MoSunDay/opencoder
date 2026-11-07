//! Secondary index maintenance and unique-constraint enforcement.
//!
//! A unique index is a KV row `idx/<table>/<column>/<encoded-value>` whose
//! value is the owning row's primary key. Checks run under the engine's
//! write lease and the index write is staged in the SAME batch as the row,
//! mirroring a SQL `UNIQUE` constraint inside one transaction: a conflict
//! aborts the batch before anything is committed.

use anyhow::{bail, Result};

use crate::rdb_store::kv::{decode, encode};

/// Read the owner recorded for an index key (`None` when free).
pub fn owner(store: &rdb::store::Store, index_key: &[u8]) -> Result<Option<Vec<u8>>> {
    rdb::store::get(store, b"", index_key).map_err(engine_err)
}

/// Reserve an index key for `row_key`, rejecting a different owner.
///
/// Mirrors the SQL failure text so callers surface the same conflict the
/// SQL backend produces (`UNIQUE constraint failed: <constraint>`).
pub fn claim_unique(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    index_key: &[u8],
    row_key: &[u8],
    constraint: &str,
) -> Result<()> {
    if let Some(existing) = owner(store, index_key)? {
        if existing.as_slice() != row_key {
            bail!("UNIQUE constraint failed: {constraint}");
        }
    }
    batch.put(index_key, row_key);
    Ok(())
}

/// Point an index key at `row_key` (overwrites any previous owner; used for
/// non-unique membership indexes whose value is read back by scanners).
pub fn put(batch: &mut rocksdb::WriteBatch, index_key: &[u8], value: &[u8]) {
    batch.put(index_key, value);
}

/// Drop an index entry in the same batch as its row delete.
pub fn remove(batch: &mut rocksdb::WriteBatch, index_key: &[u8]) {
    batch.delete(index_key);
}

/// Existence marker index value (empty owner).
pub fn marker() -> Vec<u8> {
    Vec::new()
}

/// Does any key exist under this index prefix?
pub fn any_under(store: &rdb::store::Store, prefix: &[u8]) -> bool {
    !rdb::store::prefix_keys_collect(store, prefix, 1).is_empty()
}

/// Read the owner as a UTF-8 primary key (for indexes storing string PKs).
pub fn owner_str(store: &rdb::store::Store, index_key: &[u8]) -> Result<Option<String>> {
    match owner(store, index_key)? {
        Some(bytes) => Ok(Some(
            String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("invalid index owner"))?,
        )),
        None => Ok(None),
    }
}

/// Read the owner as an `i64` primary key (for seq-valued indexes).
pub fn owner_i64(store: &rdb::store::Store, index_key: &[u8]) -> Result<Option<i64>> {
    match owner(store, index_key)? {
        Some(bytes) => Ok(Some(decode(&bytes)?)),
        None => Ok(None),
    }
}

/// Stage an `i64` owner value (compact JSON integer).
pub fn put_i64(batch: &mut rocksdb::WriteBatch, index_key: &[u8], value: i64) -> Result<()> {
    batch.put(index_key, encode(&value)?);
    Ok(())
}

fn engine_err(e: String) -> anyhow::Error {
    anyhow::anyhow!("rdb engine: {e}")
}

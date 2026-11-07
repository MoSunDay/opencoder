//! Atomic batch writes and shared write-path helpers.
//!
//! Every mutation stages all row + index + counter updates in ONE
//! `rocksdb::WriteBatch` and commits it through `rdb::store::batch_write`
//! (synchronous, atomic): a closure that fails mid-build leaves NOTHING
//! written, which is the KV equivalent of a rolled-back transaction.
//! All writers serialize on the engine's write lease, so check-then-put
//! sequences inside one closure are race-free in-process.

use anyhow::Result;

use crate::rdb_store::kv::encode;

/// Commit a staged batch atomically (map the engine's string error).
pub fn commit(store: &rdb::store::Store, batch: rocksdb::WriteBatch) -> Result<()> {
    rdb::store::batch_write(store, batch).map_err(|e| anyhow::anyhow!("rdb engine: {e}"))
}

/// Stage a JSON row put.
pub fn put<T: serde::Serialize>(
    batch: &mut rocksdb::WriteBatch,
    key: &[u8],
    value: &T,
) -> Result<()> {
    batch.put(key, encode(value)?);
    Ok(())
}

/// Stage a raw put.
pub fn put_raw(batch: &mut rocksdb::WriteBatch, key: &[u8], value: &[u8]) {
    batch.put(key, value);
}

/// Stage a delete.
pub fn del(batch: &mut rocksdb::WriteBatch, key: &[u8]) {
    batch.delete(key);
}

/// Stage the deletion of every key under `prefix` (collect-then-delete; the
/// delete_range tombstone equivalent for callers that need per-key work).
pub fn delete_prefix(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    prefix: &[u8],
) -> Result<()> {
    for key in rdb::store::prefix_keys_collect(store, prefix, 0) {
        batch.delete(&key);
    }
    Ok(())
}

/// Delete a whole child range in one tombstone (`ON DELETE CASCADE`).
pub fn delete_range(store: &rdb::store::Store, prefix: &[u8]) -> Result<()> {
    let upper = crate::rdb_store::kv::prefix_upper(prefix);
    rdb::store::delete_range(store, prefix, &upper).map_err(|e| anyhow::anyhow!("rdb engine: {e}"))
}

/// Collect `(key, value)` pairs under a prefix in ascending key order.
pub fn scan_prefix(store: &rdb::store::Store, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    rdb::store::prefix_iter_collect(store, prefix, 0)
        .map_err(|e| anyhow::anyhow!("rdb engine: {e}"))
}

/// Scan descending from `from` (exclusive when `excl_from`), stopping at the
/// first `false` return — reverse pagination over ordered child ranges.
pub fn scan_down<F>(store: &rdb::store::Store, from: &[u8], excl_from: bool, f: F) -> Result<()>
where
    F: FnMut(&[u8], &[u8]) -> bool,
{
    rdb::store::for_each_down_from(store, from, excl_from, &mut Box::new(f) as _)
        .map_err(|e| anyhow::anyhow!("rdb engine: {e}"))
}

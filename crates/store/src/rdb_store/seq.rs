//! Monotonic sequence allocation.
//!
//! Two allocation styles, both running under the engine's write lease:
//! - `next_global`: persisted counter at `seq/<family>`; the incremented
//!   value is staged in the SAME batch as the rows that use it, so a failed
//!   batch rolls the counter back too. Holes are allowed (callers only need
//!   monotonicity).
//! - `next_child` / `max_child`: per-parent ordinals recovered from the
//!   child key range itself (`<parent>/<seq:020>`), which keeps per-session
//!   message/event seqs contiguous from 1 like the SQL backend's
//!   per-session `seq` ordering.

use std::collections::HashMap;

use crate::rdb_store::kv::{decode, encode, pad_ord, seq_counter_key};

/// In-batch counter carry: `batch_write` cannot read its own staged puts,
/// so a second allocation of the same family in one batch must consult the
/// value the first allocation staged instead of the committed counter.
pub type SeqCarry = HashMap<String, i64>;

/// Allocate the next value of a persisted global counter.
///
/// The counter write is staged in `batch`, so it commits atomically with the
/// rows that consume the value. Single-allocation batches can call this
/// directly; loops that allocate twice in one batch must pass a shared
/// [`SeqCarry`] to [`next_global_carried`].
pub fn next_global(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    family: &str,
) -> anyhow::Result<i64> {
    next_global_carried(store, batch, &mut SeqCarry::new(), family)
}

/// [`next_global`] with an explicit in-batch carry shared across calls.
pub fn next_global_carried(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    carry: &mut SeqCarry,
    family: &str,
) -> anyhow::Result<i64> {
    let next = match carry.get(family) {
        Some(staged) => *staged + 1,
        None => {
            let key = seq_counter_key(family);
            let current: i64 = match rdb::store::get(store, b"", &key).map_err(rdb_engine_err)? {
                Some(bytes) => decode::<i64>(&bytes)?,
                None => 0,
            };
            current + 1
        }
    };
    carry.insert(family.to_string(), next);
    batch.put(seq_counter_key(family), encode(&next)?);
    Ok(next)
}

/// Highest ordinal already present under a child prefix (`<prefix><ord>`).
pub fn max_child(store: &rdb::store::Store, prefix: &[u8]) -> i64 {
    // Seek from the largest representable ordinal: the first key at or below
    // it that still carries the prefix IS the maximum child.
    let mut from = prefix.to_vec();
    from.extend_from_slice(pad_ord(i64::MAX).as_bytes());
    let mut max = 0i64;
    let _ = rdb::store::for_each_down_from(store, &from, false, &mut |key, _| {
        if let Some(ord) = parse_ord(key, prefix) {
            max = ord;
        }
        false // first (descending) hit decides
    });
    max
}

/// Allocate the next per-parent ordinal (`max_child + 1`).
///
/// The key itself carries the ordinal, so no counter row is needed and a
/// failed batch leaves the previous maximum untouched.
pub fn next_child(store: &rdb::store::Store, prefix: &[u8]) -> i64 {
    max_child(store, prefix) + 1
}

/// Parse the zero-padded ordinal suffix of a child key; non-matching keys
/// are skipped (returns `None`).
pub fn parse_ord(key: &[u8], prefix: &[u8]) -> Option<i64> {
    let suffix = key.strip_prefix(prefix)?;
    let text = std::str::from_utf8(suffix).ok()?;
    if text.len() != crate::rdb_store::kv::ORD_WIDTH || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Build the scan start key for ordinals strictly greater than `from`.
pub fn after_ord(prefix: &[u8], from: i64) -> Vec<u8> {
    let mut k = prefix.to_vec();
    k.extend_from_slice(pad_ord(from).as_bytes());
    k
}

fn rdb_engine_err(e: String) -> anyhow::Error {
    anyhow::anyhow!("rdb engine: {e}")
}

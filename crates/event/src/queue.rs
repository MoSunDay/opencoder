//! Durable RocksDB [`EventLedger`] for the event queue (`rdb` feature).
//!
//! One RocksDB database owns the whole queue surface (log, listeners,
//! waits, cursors), opened through the `rdb` wrapper's store slice. Key
//! layout after the wrapper's slot prefix:
//!
//! - `evt/<seq:020>`: log rows (JSON); the seq is zero-padded to 20 digits
//!   so byte order equals numeric order and replay scans are seeks;
//! - `evn/last`: last allocated seq (JSON i64), advanced in the SAME
//!   WriteBatch as the log row, so an append is one atomic commit and a
//!   crash can never reuse a seq;
//! - `evc/<consumer-id>`: durable replay cursors (JSON i64);
//! - `evl/<listener-id>`: listener specs (JSON);
//! - `evw/<execution-id>`: pause-wait kind patterns (raw bytes).
//!
//! Ownership: while an instance lives, RocksDB's database lock refuses
//! every other open of the same path (single owner across processes), and
//! the in-process write lease serializes the read-modify-write append.
//! Blocking RocksDB calls run on tokio's blocking pool, mirroring the
//! wrapper's own async twins. This module is the only RocksDB consumer in
//! the workspace; the default build of this crate stays pure.

use std::path::Path;
use std::sync::Arc;

use anyhow::{ensure, Result};
use async_trait::async_trait;
use rocksdb::WriteBatch;
use serde::de::DeserializeOwned;
use tokio::sync::Mutex;

use crate::domain::consumer::{validate_consumer_id, validate_cursor};
use crate::domain::event::{EventKind, EventRecord};
use crate::domain::listener::{validate, validate_kind_pattern, ListenerSpec};
use crate::domain::registry::matches;
use crate::ledger::EventLedger;

/// RocksDB slot every queue key lives under (`<slot>/`, the wrapper's
/// physical key layout). Any fixed slot works: this ledger owns the file.
const QUEUE_SLOT: u16 = 1;

/// Log rows: `evt/` plus a 20-digit zero-padded seq.
const EVENT_PREFIX: &str = "evt/";
/// High-water mark of allocated seqs.
const SEQ_KEY: &str = "evn/last";
/// Replay cursors, keyed by consumer id.
const CURSOR_PREFIX: &str = "evc/";
/// Listener specs, keyed by listener id.
const LISTENER_PREFIX: &str = "evl/";
/// Pause-wait patterns, keyed by execution id.
const WAIT_PREFIX: &str = "evw/";

/// Width of the zero-padded seq inside event keys.
const SEQ_WIDTH: usize = 20;

/// Full physical key for a domain suffix.
fn queue_key(suffix: &str) -> Vec<u8> {
    let mut key = rdb::store::slot_prefix(QUEUE_SLOT);
    key.extend_from_slice(suffix.as_bytes());
    key
}

/// Physical key of the log row `seq`.
fn event_key(seq: i64) -> Vec<u8> {
    queue_key(&format!("{EVENT_PREFIX}{seq:0SEQ_WIDTH$}"))
}

/// Next seq to allocate from the persisted high-water mark; `None` means
/// the log is empty and allocation starts at 1.
fn next_seq(last: Option<i64>) -> Result<i64> {
    match last {
        None => Ok(1),
        Some(i64::MAX) => Err(anyhow::anyhow!("event seq space exhausted")),
        Some(n) => Ok(n + 1),
    }
}

/// Key-safety for consumer ids on top of [`validate_consumer_id`]: the id
/// becomes persistent key bytes, so control characters and the
/// path-traversal shapes `.`/`..` are refused even though the shared rule
/// (non-empty, ascii, no whitespace, no `/`, bounded) would accept them.
fn ensure_key_safe(id: &str) -> Result<()> {
    validate_consumer_id(id).map_err(|e| anyhow::anyhow!("{e}"))?;
    ensure!(
        id.chars().all(|c| ('!'..='~').contains(&c)),
        "consumer id must be printable ascii"
    );
    ensure!(id != "." && id != "..", "consumer id must not be a dot segment");
    Ok(())
}

/// Runs one blocking RocksDB closure on tokio's blocking pool.
async fn blocking<T, F>(store: Arc<rdb::store::Store>, work: F) -> Result<T>
where
    F: FnOnce(&rdb::store::Store) -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(move || work(&store))
        .await
        .map_err(|e| anyhow::anyhow!("queue worker panicked: {e}"))?
}

fn encode_json<T: serde::Serialize>(value: &T) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(value)?)
}

fn decode_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    Ok(serde_json::from_slice(bytes)?)
}

/// One JSON row by full physical key; `None` when absent.
fn read_json<T: DeserializeOwned>(store: &rdb::store::Store, key: &[u8]) -> Result<Option<T>> {
    rdb::store::ops::get_physical(store, key)
        .map_err(|e| anyhow::anyhow!("read key {key:?}: {e}"))?
        .map(|bytes| decode_json(&bytes))
        .transpose()
}

/// Decoded rows under one prefix, in key order. The ordered walk stops at
/// the first key outside the prefix; decode failures surface as errors
/// (corrupt rows are never silently dropped).
fn scan_prefix_json<T: DeserializeOwned>(
    store: &rdb::store::Store,
    prefix: &[u8],
) -> Result<Vec<T>> {
    let mut values: Vec<Vec<u8>> = Vec::new();
    rdb::store::ops::for_each_from(store, prefix, false, &mut |key, value| {
        if !key.starts_with(prefix) {
            return false;
        }
        values.push(value.to_vec());
        true
    })
    .map_err(|e| anyhow::anyhow!("scan prefix {prefix:?}: {e}"))?;
    values.iter().map(|bytes| decode_json(bytes)).collect()
}

/// Durable [`EventLedger`] over one RocksDB database.
///
/// Open with [`RdbQueueLedger::open`]. All mutations are single atomic
/// WriteBatch commits (fsync-ed by the wrapper); appends additionally
/// hold the write lease because they read the seq counter before writing.
pub struct RdbQueueLedger {
    store: Arc<rdb::store::Store>,
    write_lease: Mutex<()>,
}

impl RdbQueueLedger {
    /// Opens (or creates) the queue database at `path`. A second open of
    /// the same path fails while this instance is alive.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path
            .as_ref()
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("queue path must be valid utf-8"))?;
        let store =
            rdb::store::rocksdb::open(path).map_err(|e| anyhow::anyhow!("open {path}: {e}"))?;
        Ok(Self {
            store: Arc::new(store),
            write_lease: Mutex::new(()),
        })
    }

    /// Last allocated log seq (0 when the log is empty).
    pub async fn last_seq(&self) -> Result<i64> {
        let key = queue_key(SEQ_KEY);
        blocking(Arc::clone(&self.store), move |store| {
            Ok(read_json::<i64>(store, &key)?.unwrap_or(0))
        })
        .await
    }

    /// Commits one batch atomically (sync write, the wrapper's discipline).
    async fn commit(&self, batch: WriteBatch) -> Result<()> {
        blocking(Arc::clone(&self.store), move |store| {
            rdb::store::ops::batch_write(store, batch)
                .map_err(|e| anyhow::anyhow!("queue commit: {e}"))
        })
        .await
    }
}

#[async_trait]
impl EventLedger for RdbQueueLedger {
    async fn append(&self, record: &EventRecord) -> Result<()> {
        ensure!(!record.id.trim().is_empty(), "event id must be non-empty");
        ensure!(
            !record.kind.as_str().trim().is_empty(),
            "event kind must be non-empty"
        );
        // The lease spans the counter read and its batched update, so
        // concurrent appends can never observe the same high-water mark.
        let _lease = self.write_lease.lock().await;

        let seq_key = queue_key(SEQ_KEY);
        let last = blocking(Arc::clone(&self.store), move |store| {
            read_json::<i64>(store, &seq_key)
        })
        .await?;
        let seq = next_seq(last)?;

        // The ledger owns ordering: the stored row keeps the record's
        // identity but carries the allocated seq, so replay cursors stay
        // monotonic and duplicates can never overwrite each other.
        let mut stored = record.clone();
        stored.seq = seq;

        let mut batch = WriteBatch::default();
        batch.put(event_key(seq), encode_json(&stored)?);
        batch.put(queue_key(SEQ_KEY), encode_json(&seq)?);
        self.commit(batch).await
    }

    async fn events_after(&self, after_seq: i64, limit: u32) -> Result<Vec<EventRecord>> {
        if limit == 0 || after_seq == i64::MAX {
            return Ok(Vec::new());
        }
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let prefix = queue_key(EVENT_PREFIX);
        let from = event_key(after_seq + 1);
        blocking(Arc::clone(&self.store), move |store| {
            let mut rows: Vec<Vec<u8>> = Vec::new();
            rdb::store::ops::for_each_from(store, &from, false, &mut |key, value| {
                if !key.starts_with(&prefix) {
                    return false;
                }
                rows.push(value.to_vec());
                rows.len() < limit
            })
            .map_err(|e| anyhow::anyhow!("event scan: {e}"))?;
            rows.iter().map(|bytes| decode_json(bytes)).collect()
        })
        .await
    }

    async fn save_listener(&self, spec: &ListenerSpec) -> Result<()> {
        validate(spec)?;
        let mut batch = WriteBatch::default();
        batch.put(
            queue_key(&format!("{LISTENER_PREFIX}{}", spec.id)),
            encode_json(spec)?,
        );
        self.commit(batch).await
    }

    async fn delete_listener(&self, id: &str) -> anyhow::Result<()> {
        let mut batch = WriteBatch::default();
        batch.delete(queue_key(&format!("{LISTENER_PREFIX}{id}")));
        self.commit(batch).await
    }

    async fn list_listeners(&self) -> anyhow::Result<Vec<ListenerSpec>> {
        let prefix = queue_key(LISTENER_PREFIX);
        let mut specs: Vec<ListenerSpec> =
            blocking(Arc::clone(&self.store), move |store| {
                scan_prefix_json::<ListenerSpec>(store, &prefix)
            })
            .await?;
        specs.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(specs)
    }

    async fn save_wait(&self, execution_id: &str, kind_pattern: &str) -> Result<()> {
        ensure!(
            !execution_id.trim().is_empty(),
            "execution id must be non-empty"
        );
        validate_kind_pattern(kind_pattern)?;
        let mut batch = WriteBatch::default();
        batch.put(
            queue_key(&format!("{WAIT_PREFIX}{execution_id}")),
            kind_pattern.as_bytes(),
        );
        self.commit(batch).await
    }

    async fn clear_wait(&self, execution_id: &str) -> anyhow::Result<()> {
        let mut batch = WriteBatch::default();
        batch.delete(queue_key(&format!("{WAIT_PREFIX}{execution_id}")));
        self.commit(batch).await
    }

    async fn due_waits(&self, kind: &EventKind) -> anyhow::Result<Vec<String>> {
        let prefix = queue_key(WAIT_PREFIX);
        let kind = kind.as_str().to_string();
        blocking(Arc::clone(&self.store), move |store| {
            let mut waits: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
            rdb::store::ops::for_each_from(store, &prefix, false, &mut |key, value| {
                if !key.starts_with(&prefix) {
                    return false;
                }
                waits.push((key.to_vec(), value.to_vec()));
                true
            })
            .map_err(|e| anyhow::anyhow!("wait scan: {e}"))?;
            let mut due: Vec<String> = waits
                .into_iter()
                .filter(|(_, pattern)| {
                    matches(&String::from_utf8_lossy(pattern), &kind)
                })
                .map(|(key, _)| String::from_utf8_lossy(&key[prefix.len()..]).into_owned())
                .collect();
            due.sort();
            Ok(due)
        })
        .await
    }

    async fn consumer_cursor(&self, consumer_id: &str) -> Result<Option<i64>> {
        ensure_key_safe(consumer_id)?;
        let key = queue_key(&format!("{CURSOR_PREFIX}{consumer_id}"));
        blocking(Arc::clone(&self.store), move |store| {
            read_json::<i64>(store, &key)
        })
        .await
    }

    async fn save_consumer_cursor(&self, consumer_id: &str, seq: i64) -> Result<()> {
        ensure_key_safe(consumer_id)?;
        validate_cursor(seq).map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut batch = WriteBatch::default();
        batch.put(
            queue_key(&format!("{CURSOR_PREFIX}{consumer_id}")),
            encode_json(&seq)?,
        );
        self.commit(batch).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_keys_pad_so_byte_order_equals_seq_order() {
        assert!(event_key(1) < event_key(2));
        assert!(event_key(9) < event_key(10));
        assert!(event_key(999) < event_key(1_000_000_000));
        let prefix = queue_key(EVENT_PREFIX);
        assert!(event_key(i64::MAX).starts_with(&prefix));
    }

    #[test]
    fn next_seq_starts_at_one_and_refuses_exhaustion() {
        assert_eq!(next_seq(None).unwrap(), 1);
        assert_eq!(next_seq(Some(3)).unwrap(), 4);
        assert!(next_seq(Some(i64::MAX)).is_err());
    }

    #[test]
    fn key_safety_rejects_control_chars_and_dot_segments() {
        assert!(ensure_key_safe("pump-1").is_ok());
        for bad in ["\u{1}", "\u{7f}", ".", "..", "a/../b"] {
            assert!(ensure_key_safe(bad).is_err(), "{bad:?}");
        }
    }
}

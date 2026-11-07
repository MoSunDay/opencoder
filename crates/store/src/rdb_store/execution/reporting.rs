//! Reporting outbox (`execution_report_*` tables) over the rdb KV engine.
//!
//! Key families (components are ULID-style ids or fixed action names, so
//! plain `/` nesting is collision-free):
//! - `brq/<key>`                queue row; `<key>` is the JSON encoding of
//!   `(execution_id, action, item)` — the SQL primary key, and the ordering
//!   the `pending_reports(after, limit)` wrap-around cursor walks
//! - `bri/<execution_id>/<key>` queue-by-execution index (the DELETE ranges)
//! - `brk/<execution_id>/<action>/<item>`  acknowledgement cursor rows
//! - `brr/<run_id>`             per-run projection watermark
//! - `brf`                      report format version (single row)
//!
//! Mutations stage every row + index change in ONE atomic batch, so a
//! failed write leaves the outbox untouched — the KV form of the SQL
//! transaction that keeps brain commits and their report projection
//! consistent.

mod queue;
mod read;

pub use queue::{acknowledge, recover, update};
pub use read::{acks, has_pending, operation, pending, state};

use anyhow::{ensure, Result};

use crate::rdb_store::kv;
use crate::rdb_store::tx;

/// Queue row keyed by the SQL primary key (JSON `(id, action, item)`).
pub(super) fn queue_key(execution_id: &str, action: &str, item: &str) -> String {
    serde_json::to_string(&(execution_id, action, item)).expect("JSON tuple key is total")
}

pub(super) fn queue_row_key(key: &str) -> Vec<u8> {
    format!("brq/{key}").into_bytes()
}

/// Queue membership by execution id (value = the queue row key suffix).
pub(super) fn queue_index_key(execution_id: &str, key: &str) -> Vec<u8> {
    format!("bri/{execution_id}/{key}").into_bytes()
}

pub(super) fn queue_index_prefix(execution_id: &str) -> Vec<u8> {
    format!("bri/{execution_id}/").into_bytes()
}

pub(super) fn ack_key(execution_id: &str, action: &str, item: &str) -> Vec<u8> {
    format!("brk/{execution_id}/{action}/{item}").into_bytes()
}

pub(super) fn watermark_key(run_id: &str) -> Vec<u8> {
    format!("brr/{run_id}").into_bytes()
}

const FORMAT_KEY: &[u8] = b"brf";

/// Reader/writer format fence: a store may only be opened by readers at
/// least as new as the persisted version, and only write versions at least
/// as new as it (ratchets forward, never backward).
pub fn format(store: &rdb::store::Store, reader: u32, writer: u32) -> Result<()> {
    ensure!(writer <= reader, "report writer exceeds reader");
    let stored = kv::get_json::<i64>(store, FORMAT_KEY)?.unwrap_or(0);
    ensure!(
        stored <= reader as i64,
        "report state requires a newer Runtime reader"
    );
    if writer as i64 > stored {
        let mut batch = rocksdb::WriteBatch::default();
        tx::put(&mut batch, FORMAT_KEY, &(writer as i64))?;
        tx::commit(store, batch)?;
    }
    Ok(())
}

//! Outbox reads: pending selection with the wrap-around cursor,
//! acknowledgements, per-run watermark and single-operation lookups.

use anyhow::Result;

use crate::rdb_store::execution::brain_layered;
use crate::rdb_store::execution::reporting::queue::QueueRow;
use crate::rdb_store::execution::reporting::watermark_key;
use crate::rdb_store::{kv, tx};
use crate::reporting::{PendingReport, ReportAck, ReportRunState};

const QUEUE_PREFIX: &[u8] = b"brq/";
const ACK_PREFIX: &str = "brk/";

pub fn has_pending(store: &rdb::store::Store) -> Result<bool> {
    Ok(!rdb::store::prefix_keys_collect(store, QUEUE_PREFIX, 1).is_empty())
}

/// Eligible rows in key order. Two ranges wrap the cursor exactly like the
/// SQL `key > ?` / `key <= ?` pair; a row that fails to decode fails the
/// read (the SQL payload parse propagates too).
pub fn pending(
    store: &rdb::store::Store,
    after: Option<&str>,
    limit: u32,
) -> Result<Vec<PendingReport>> {
    let limit = limit.clamp(1, 64) as usize;
    let mut result = Vec::new();
    let after_key = after.map(|a| format!("brq/{a}").into_bytes());
    let from = after_key.clone().unwrap_or_else(|| QUEUE_PREFIX.to_vec());
    scan(store, &from, None, limit, &mut result)?;
    if let Some(after_key) = &after_key {
        if result.len() < limit {
            scan(store, QUEUE_PREFIX, Some(after_key), limit, &mut result)?;
        }
    }
    Ok(result)
}

/// Visit eligible rows from `from` upward, stopping at `stop_inclusive`
/// (the wrap point) or once `limit` rows are collected.
fn scan(
    store: &rdb::store::Store,
    from: &[u8],
    stop_inclusive: Option<&[u8]>,
    limit: usize,
    out: &mut Vec<PendingReport>,
) -> Result<()> {
    let mut failure = None;
    rdb::store::for_each_from(store, from, true, &mut |key, value| {
        if !key.starts_with(QUEUE_PREFIX)
            || out.len() >= limit
            || stop_inclusive.is_some_and(|stop| key > stop)
        {
            return false;
        }
        match kv::decode::<QueueRow>(value) {
            Ok(row) => {
                if row.eligible {
                    out.push(row.into());
                }
                out.len() < limit
            }
            Err(error) => {
                failure = Some(error);
                false
            }
        }
    })
    .map_err(kv::engine_err)?;
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(())
}

pub fn acks(store: &rdb::store::Store) -> Result<Vec<ReportAck>> {
    let mut result = Vec::new();
    for (key, value) in tx::scan_prefix(store, ACK_PREFIX.as_bytes())? {
        let Some(suffix) = key
            .strip_prefix(ACK_PREFIX.as_bytes())
            .and_then(|k| std::str::from_utf8(k).ok())
        else {
            continue;
        };
        // `<execution_id>/<action>/<item>` (item may be empty)
        let mut parts = suffix.splitn(3, '/');
        if let (Some(execution_id), Some(action), Some(item)) =
            (parts.next(), parts.next(), parts.next())
        {
            result.push(ReportAck {
                execution_id: execution_id.to_owned(),
                action: action.to_owned(),
                item: item.to_owned(),
                cursor: kv::decode::<i64>(&value)? as u64,
            });
        }
    }
    Ok(result)
}

pub fn state(store: &rdb::store::Store, id: &str) -> Result<Option<ReportRunState>> {
    Ok(kv::get_json::<super::queue::WatermarkRow>(store, &watermark_key(id))?.map(Into::into))
}

pub fn operation(
    store: &rdb::store::Store,
    id: &str,
    operation: &str,
) -> Result<Option<opencoder_core::brain::layered::LayeredOperation>> {
    brain_layered::operation(store, id, operation)
}

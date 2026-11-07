//! Session events (`sev/<session>/<seq:020>`): atomic batch append, replay
//! cursor queries and bounded payload reads.
//!
//! Per-session seqs are contiguous from 1 (allocated under the write
//! lease), which preserves the SSE replay contract (`events_after(after)`,
//! `last_event_seq`) the SQL AUTOINCREMENT column provided.

use anyhow::{bail, Context, Result};

use crate::rdb_store::kv::{decode, event_key, event_prefix};
use crate::rdb_store::{seq, tx};
use crate::types::{SessionEventPage, SessionEventRecord};

/// Per-row accounting overhead applied against a page's payload budget.
const EVENT_OVERHEAD_BYTES: usize = 512;

/// Persist one batch atomically; seqs are returned in emission order.
pub fn append_many(store: &rdb::store::Store, events: &[SessionEventRecord]) -> Result<Vec<i64>> {
    if events.is_empty() {
        return Ok(Vec::new());
    }
    let session_id = events[0].session_id.as_str();
    if super::sessions::get(store, session_id)?.is_none() {
        bail!("append_events: session {session_id} not found");
    }
    for ev in events {
        if ev.session_id != session_id {
            bail!("append_events: all events in a batch must share the same session_id");
        }
    }
    let prefix = event_prefix(session_id);
    let mut batch = rocksdb::WriteBatch::default();
    let mut seqs = Vec::with_capacity(events.len());
    for (next, ev) in (seq::next_child(store, &prefix)..).zip(events) {
        // The stored row is the record itself; `seq` becomes the row ordinal.
        tx::put(&mut batch, &event_key(session_id, next), ev)?;
        seqs.push(next);
    }
    tx::commit(store, batch)?;
    Ok(seqs)
}

/// Rows after `after_seq` in ascending order.
pub fn after(
    store: &rdb::store::Store,
    session_id: &str,
    after_seq: i64,
) -> Result<Vec<SessionEventRecord>> {
    let mut out = Vec::new();
    for (key, value) in tx::scan_prefix(store, &event_prefix(session_id))? {
        let Some(s) = seq::parse_ord(&key, &event_prefix(session_id)) else {
            continue;
        };
        if s <= after_seq {
            continue;
        }
        let mut rec: SessionEventRecord = decode(&value)?;
        rec.seq = Some(s);
        out.push(rec);
    }
    Ok(out)
}

/// Highest event seq of a session (0 when none).
pub fn last_seq(store: &rdb::store::Store, session_id: &str) -> Result<i64> {
    Ok(seq::max_child(store, &event_prefix(session_id)))
}

/// Bounded event page: row cap plus payload byte budget.
pub fn page(
    store: &rdb::store::Store,
    session_id: &str,
    after_seq: i64,
    limit: u32,
    payload_budget: usize,
) -> Result<SessionEventPage> {
    let limit = limit.clamp(1, 200) as usize;
    let mut metas: Vec<(SessionEventRecord, usize)> = Vec::new();
    for rec in after(store, session_id, after_seq)? {
        let len = serde_json::to_vec(&rec.payload)
            .map(|v| v.len())
            .unwrap_or(0);
        metas.push((rec, len));
        if metas.len() > limit {
            break;
        }
    }
    let total_metas = metas.len();
    let mut events = Vec::with_capacity(limit.min(total_metas));
    let mut used = 0usize;
    for (rec, payload_len) in metas.into_iter().take(limit) {
        let row_bytes = payload_len.saturating_add(EVENT_OVERHEAD_BYTES);
        if row_bytes > payload_budget && events.is_empty() {
            let seq = rec.seq;
            events.push(SessionEventRecord {
                session_id: session_id.to_owned(),
                kind: rec.kind,
                payload: serde_json::json!({
                    "omitted": true,
                    "reason": "event_payload_exceeds_page_budget",
                    "total_bytes": payload_len,
                    "read_via": "event_payload",
                }),
                ts: rec.ts,
                seq,
                sse_kind: rec.sse_kind,
            });
            break;
        }
        if used.saturating_add(row_bytes) > payload_budget {
            break;
        }
        events.push(rec);
        used += row_bytes;
    }
    Ok(SessionEventPage {
        more: events.len() < total_metas,
        events,
    })
}

/// Bounded payload byte slice of one event.
pub fn payload_chunk(
    store: &rdb::store::Store,
    session_id: &str,
    s: i64,
    offset: u64,
    max_bytes: usize,
) -> Result<Option<crate::types::PayloadChunkRecord>> {
    let Some(rec) = kv_get_event(store, session_id, s)? else {
        return Ok(None);
    };
    let payload = serde_json::to_vec(&rec.payload).context("serialize event payload")?;
    let total = payload.len() as u64;
    if offset > total {
        bail!("event payload offset exceeds total bytes");
    }
    let take = max_bytes.clamp(1, 64 * 1024);
    let start = offset as usize;
    let end = (start + take).min(payload.len());
    Ok(Some(crate::types::PayloadChunkRecord {
        total_bytes: total,
        bytes: payload[start..end].to_vec(),
    }))
}

fn kv_get_event(
    store: &rdb::store::Store,
    session_id: &str,
    s: i64,
) -> Result<Option<SessionEventRecord>> {
    let mut rec =
        crate::rdb_store::kv::get_json::<SessionEventRecord>(store, &event_key(session_id, s))?;
    if let Some(rec) = rec.as_mut() {
        rec.seq = Some(s);
    }
    Ok(rec)
}

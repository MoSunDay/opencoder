//! DAG event stream (`dag_events`) over the rdb KV engine — the
//! node-uploaded, append-only log the browser UI projects step state from.
//! The server keeps NO per-step scheduling state: it stores and replays.
//!
//! Key family: `dgev/<seq:020>` — one row per event, keyed by the GLOBAL
//! sequence number (`seq/dag-events` persisted counter, allocated in the
//! same batch as the rows). The key order IS the emission order, so the
//! SSE `Last-Event-ID` cursor (`dag_events_after`) is a plain range scan.

use anyhow::Result;

use crate::rdb_store::kv::{self, pad_ord};
use crate::rdb_store::{seq, tx};
use crate::types::DagEventRecord;

/// Persisted global counter family (the AUTOINCREMENT stand-in).
pub(crate) const SEQ_FAMILY: &str = "dag-events";

fn event_prefix() -> Vec<u8> {
    b"dgev/".to_vec()
}

fn event_key(seq: i64) -> Vec<u8> {
    let mut k = event_prefix();
    k.extend_from_slice(pad_ord(seq).as_bytes());
    k
}

/// Persist a batch of node-uploaded events in ONE batch; the returned seqs
/// are the assigned global ids in emission (ascending) order.
pub fn append_events(store: &rdb::store::Store, events: &[DagEventRecord]) -> Result<Vec<i64>> {
    if events.is_empty() {
        return Ok(Vec::new());
    }
    let mut batch = rocksdb::WriteBatch::default();
    let mut carry = seq::SeqCarry::new();
    let mut seqs = Vec::with_capacity(events.len());
    for ev in events {
        let seq = seq::next_global_carried(store, &mut batch, &mut carry, SEQ_FAMILY)?;
        seqs.push(seq);
        let row = DagEventRecord {
            seq: Some(seq),
            run_id: ev.run_id.clone(),
            kind: ev.kind.clone(),
            step: ev.step.clone(),
            payload: ev.payload.clone(),
            at_ms: ev.at_ms,
        };
        tx::put(&mut batch, &event_key(seq), &row)?;
    }
    tx::commit(store, batch)?;
    Ok(seqs)
}

/// Stage the synthetic `run_finished` frame for `run_id` inside the caller's
/// batch and return its seq — shared by `finalize_dag_run` and the
/// lost-node sweep so the frame lands in the SAME commit as the status flip.
/// Payload matches the persisted row: `{"status": ...}` + optional `"error"`.
pub(crate) fn stage_run_finished(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    carry: &mut seq::SeqCarry,
    run_id: &str,
    status: &str,
    error: Option<&str>,
    at_ms: i64,
) -> Result<i64> {
    let mut payload = serde_json::json!({ "status": status });
    if let Some(err) = error {
        payload["error"] = serde_json::json!(err);
    }
    let seq = seq::next_global_carried(store, batch, carry, SEQ_FAMILY)?;
    let row = DagEventRecord {
        seq: Some(seq),
        run_id: run_id.to_string(),
        kind: "run_finished".to_string(),
        step: None,
        payload,
        at_ms,
    };
    tx::put(batch, &event_key(seq), &row)?;
    Ok(seq)
}

/// Replay slice for the run SSE: `seq > after`, ascending, bounded —
/// strictly-after semantics for `Last-Event-ID` resumption.
pub fn events_after(
    store: &rdb::store::Store,
    run_id: &str,
    after: i64,
    limit: u32,
) -> Result<Vec<DagEventRecord>> {
    let prefix = event_prefix();
    let mut from = prefix.clone();
    from.extend_from_slice(pad_ord(after).as_bytes());
    let mut out = Vec::new();
    rdb::store::for_each_from(store, &from, true, &mut |key, value| {
        if !key.starts_with(&prefix) || out.len() >= limit as usize {
            return false; // left the event range or filled the window — stop
        }
        let Ok(row) = kv::decode::<DagEventRecord>(value) else {
            return true; // skip undecodable rows, keep scanning
        };
        if row.run_id == run_id {
            out.push(row);
        }
        true
    })
    .map_err(kv::engine_err)?;
    Ok(out)
}

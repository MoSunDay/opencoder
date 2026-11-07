//! Frozen DAG step checkpoints: the latest lifecycle row per step plus the
//! exact session event cursor — never step-log payloads.
//!
//! The SQL version selects `max(seq)` and `max(step_started.at_ms)` per
//! step with a GROUP BY; the KV version streams the session's event rows
//! once (`sev/<session>/<seq:020>`), keeping only the per-step latest row
//! and the started watermark, so the result depends on step count, not on
//! log volume.

use anyhow::Result;

use crate::rdb_store::conversation::events;
use crate::rdb_store::kv;
use crate::rdb_store::tx;
use crate::store::dag_snapshot::{DagStepEvent, DagStepSnapshot};
use crate::types::SessionEventRecord;

/// Latest lifecycle row per step name, ordered by that row's seq.
pub fn read(store: &rdb::store::Store, id: &str) -> Result<DagStepSnapshot> {
    let head_seq = events::last_seq(store, id)?;
    let prefix = kv::event_prefix(id);
    // (name → (seq, started, at_ms, ok, error)), plus the started watermark.
    let mut latest: std::collections::BTreeMap<String, StepState> = Default::default();
    for (key, value) in tx::scan_prefix(store, &prefix)? {
        let Some(seq) = crate::rdb_store::seq::parse_ord(&key, &prefix) else {
            continue;
        };
        let event: SessionEventRecord = kv::decode(&value)?;
        let Some(kind) = event.sse_kind.as_deref() else {
            continue;
        };
        let started = match kind {
            "step_started" => true,
            "step_done" => false,
            _ => continue,
        };
        let Some(name) = step_name(&event) else {
            continue;
        };
        let at_ms = event
            .payload
            .get("at_ms")
            .and_then(|v| v.as_i64())
            .unwrap_or(event.ts);
        let entry = latest.entry(name).or_default();
        if started && at_ms > entry.started_at_ms {
            entry.started_at_ms = at_ms;
        }
        if seq >= entry.seq {
            entry.seq = seq;
            entry.started = started;
            entry.at_ms = at_ms;
            entry.ok = event
                .payload
                .pointer("/payload/ok")
                .map(json_truthy)
                .unwrap_or(true);
            entry.error = event
                .payload
                .pointer("/payload/error")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
        }
    }
    let mut steps: Vec<DagStepEvent> = latest
        .into_iter()
        .map(|(name, state)| DagStepEvent {
            name,
            seq: state.seq,
            started: state.started,
            at_ms: state.at_ms,
            started_at_ms: state.started_at_ms,
            ok: state.ok,
            error: state.error,
        })
        .collect();
    steps.sort_by_key(|step| step.seq);
    Ok(DagStepSnapshot { head_seq, steps })
}

#[derive(Default)]
struct StepState {
    seq: i64,
    started: bool,
    at_ms: i64,
    started_at_ms: i64,
    ok: bool,
    error: Option<String>,
}

fn step_name(event: &SessionEventRecord) -> Option<String> {
    event
        .payload
        .get("step")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
}

/// `json_extract('$.payload.ok')` truthiness: booleans and integers alike.
fn json_truthy(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Bool(v) => *v,
        v => v.as_i64().unwrap_or(1) != 0,
    }
}

//! Brain v4 layered projection (`brain_layered_runs` / `_operations` /
//! `_events`) over the rdb KV engine.
//!
//! Key families:
//! - `blr/<run_id>`                          run row (JSON `LayeredRun`)
//! - `blo/<execution_id>`                    operation row (JSON `LayeredOperation`)
//! - `bli/<run_id>/<layer:010>/<operation_id>`  run membership, ordered by
//!   `(layer, operation_id)` — the SQL `ORDER BY layer, operation_id` read
//! - `blu/<operation_id>`                    `UNIQUE(operation_id)` → execution_id
//! - `ble/<run_id>/<seq:020>`                event log row (JSON `LayeredEvent`)
//! - `bleu/<run_id>/<execution_id>/<seq:020>` `UNIQUE(run_id, execution_id,
//!   source_sequence)` wake-receipt dedup marker
//!
//! `commit` validates the change against the CURRENT committed state under
//! the write lease and applies projection + event log + reporting outbox in
//! ONE atomic batch, so a rejected change leaves nothing behind — the KV
//! form of the SQL `BEGIN IMMEDIATE` transaction with its unique
//! constraints.

use anyhow::{bail, ensure, Context, Result};
use opencoder_core::brain::layered::{
    LayeredChange, LayeredEvent, LayeredOperation, LayeredRun, LayeredSnapshot,
    LAYERED_SCHEMA_VERSION,
};
use std::collections::HashSet;

use crate::rdb_store::index;
use crate::rdb_store::kv::{self, pad_ord};
use crate::rdb_store::tx;

#[path = "brain_layered/validate.rs"]
mod validate;

/// Zero-padded width of the `layer` index component.
const LAYER_WIDTH: usize = 10;

pub fn run_row_key(run_id: &str) -> Vec<u8> {
    format!("blr/{run_id}").into_bytes()
}

pub fn op_row_key(execution_id: &str) -> Vec<u8> {
    format!("blo/{execution_id}").into_bytes()
}

fn op_index_key(run_id: &str, layer: u32, operation_id: &str) -> Vec<u8> {
    format!("bli/{run_id}/{layer:0LAYER_WIDTH$}/{operation_id}").into_bytes()
}

fn op_unique_index(operation_id: &str) -> Vec<u8> {
    format!("blu/{operation_id}").into_bytes()
}

pub fn event_prefix(run_id: &str) -> Vec<u8> {
    format!("ble/{run_id}/").into_bytes()
}

fn event_key(run_id: &str, ev_seq: u64) -> Vec<u8> {
    let mut key = event_prefix(run_id);
    key.extend_from_slice(pad_ord(ev_seq as i64).as_bytes());
    key
}

fn event_dedup_key(run_id: &str, execution_id: &str, source_sequence: u64) -> Vec<u8> {
    let mut key = format!("bleu/{run_id}/{execution_id}/").into_bytes();
    key.extend_from_slice(pad_ord(source_sequence as i64).as_bytes());
    key
}

/// Recompute the atomic projection from the stored run + operation rows.
/// Layer data is never cached: every read walks the membership index.
pub fn load(store: &rdb::store::Store, id: &str) -> Result<Option<LayeredSnapshot>> {
    let Some(run) = kv::get_json::<LayeredRun>(store, &run_row_key(id))? else {
        return Ok(None);
    };
    let prefix = format!("bli/{id}/").into_bytes();
    let mut operations = Vec::new();
    for (_, value) in tx::scan_prefix(store, &prefix)? {
        let execution_id = String::from_utf8(value)
            .map_err(|_| anyhow::anyhow!("invalid layered operation index owner"))?;
        if let Some(op) = kv::get_json::<LayeredOperation>(store, &op_row_key(&execution_id))? {
            operations.push(op);
        }
    }
    Ok(Some(LayeredSnapshot {
        schema_version: LAYERED_SCHEMA_VERSION,
        run,
        operations,
    }))
}

/// Events after `after` in ascending seq order (replay cursor query).
pub fn page(
    store: &rdb::store::Store,
    id: &str,
    after: u64,
    limit: u32,
) -> Result<Vec<LayeredEvent>> {
    let limit = limit.clamp(1, 500) as usize;
    let prefix = event_prefix(id);
    let mut from = prefix.clone();
    from.extend_from_slice(pad_ord(after as i64).as_bytes());
    let mut events = Vec::new();
    let mut failure = None;
    rdb::store::for_each_from(store, &from, true, &mut |key, value| {
        if !key.starts_with(prefix.as_slice()) || events.len() >= limit {
            return false;
        }
        match kv::decode::<LayeredEvent>(value) {
            Ok(event) => {
                events.push(event);
                events.len() < limit
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
    Ok(events)
}

/// One operation row of a run, addressed by `operation_id`.
pub fn operation(
    store: &rdb::store::Store,
    run_id: &str,
    operation_id: &str,
) -> Result<Option<LayeredOperation>> {
    let prefix = format!("bli/{run_id}/").into_bytes();
    let wanted = format!("/{operation_id}").into_bytes();
    for (key, _) in tx::scan_prefix(store, &prefix)? {
        let Some(suffix) = key.strip_prefix(prefix.as_slice()) else {
            continue;
        };
        // `<layer:010>/<operation_id>`
        let matches = suffix.len() > LAYER_WIDTH + 1
            && suffix[..LAYER_WIDTH].iter().all(u8::is_ascii_digit)
            && suffix[LAYER_WIDTH..] == wanted[..];
        if !matches {
            continue;
        }
        let value = rdb::store::get(store, b"", &key).map_err(kv::engine_err)?;
        if let Some(bytes) = value {
            let execution_id = String::from_utf8(bytes)
                .map_err(|_| anyhow::anyhow!("invalid layered operation index owner"))?;
            return kv::get_json::<LayeredOperation>(store, &op_row_key(&execution_id));
        }
    }
    Ok(None)
}

/// Append-only commit with generation fencing; the whole change (projection,
/// event log, dedup index, reporting outbox) lands in one atomic batch.
pub fn commit(store: &rdb::store::Store, change: &LayeredChange) -> Result<LayeredSnapshot> {
    let old = load(store, &change.run.run_id)?;
    validate::check(&old, change)?;
    let mut run = change.run.clone();
    run.last_event_seq = old.as_ref().map_or(0, |s| s.run.last_event_seq);
    let mut batch = rocksdb::WriteBatch::default();
    for op in &change.operations {
        ensure_operation(op, &run)?;
        stage_operation(store, &mut batch, op)?;
    }
    let mut staged_dedup: HashSet<Vec<u8>> = HashSet::new();
    let mut committed_events = Vec::new();
    for event in &change.events {
        ensure_event(event, &run)?;
        let mut event = event.clone();
        run.last_event_seq += 1;
        event.seq = run.last_event_seq;
        stage_event(store, &mut batch, &mut staged_dedup, &event)?;
        committed_events.push(event);
    }
    tx::put(&mut batch, &run_row_key(&run.run_id), &run)?;
    super::reporting::update(
        store,
        &mut batch,
        &run,
        &change.operations,
        &committed_events,
    )?;
    stage_platform_events(store, &mut batch, &run, &committed_events)?;
    tx::commit(store, batch)?;
    load(store, &run.run_id)?.context("layered projection disappeared")
}

/// Mirror brain layer barriers and run control signals into the platform
/// event log in the same atomic batch, so a paused dispatcher can replay them
/// once resumed.
fn stage_platform_events(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    run: &LayeredRun,
    events: &[LayeredEvent],
) -> Result<()> {
    let mut carry = crate::rdb_store::seq::SeqCarry::new();
    for event in events {
        let Some((kind, payload)) = platform_signal(run, event)? else {
            continue;
        };
        let occurred_at =
            chrono::DateTime::from_timestamp_millis(event.at_ms).unwrap_or_else(chrono::Utc::now);
        let record = opencoder_event::domain::event::rebuild_record(
            opencoder_event::domain::event::new_id(),
            kind,
            payload,
            "brain",
            0,
            occurred_at,
        );
        crate::rdb_store::events::stage_append(store, batch, &mut carry, &record)?;
    }
    Ok(())
}

/// Map a brain event onto the platform vocabulary; unrelated events yield
/// `None`. Additive mirror only — the brain projection stays authoritative.
/// `execution.paused` stays reserved for events carrying an actual execution
/// id, so run-level control maps onto the `brain.run_*` kinds; those mirrors
/// carry the committed run coordinates (`layer` / `round` / `activation`).
/// An `operation_terminal` mirror names a concrete sub-execution, so a
/// terminal event without `execution_id` rejects the whole batch instead of
/// emitting `execution_id: null`.
fn platform_signal(
    run: &LayeredRun,
    event: &LayeredEvent,
) -> Result<Option<(opencoder_event::EventKind, serde_json::Value)>> {
    let kind = match event.event_type.as_str() {
        "layer_barrier_reached" => opencoder_event::domain::event::BRAIN_LAYER_BARRIER_REACHED,
        "run_paused" => opencoder_event::domain::event::BRAIN_RUN_PAUSED,
        "run_resumed" => opencoder_event::domain::event::BRAIN_RUN_RESUMED,
        "operation_terminal" => {
            ensure!(
                event.execution_id.is_some(),
                "operation terminal event requires an execution id"
            );
            opencoder_event::domain::event::EXECUTION_TERMINAL
        }
        _ => return Ok(None),
    };
    Ok(Some((
        opencoder_event::EventKind::new(kind),
        serde_json::json!({
            "run_id": run.run_id,
            "layer": run.layer,
            "round": run.round,
            "activation": run.activation,
            "brain_event_seq": event.seq,
            "execution_id": event.execution_id,
            "execution_kind": event.execution_kind,
            "status": event.decision_summary,
        }),
    )))
}

fn ensure_operation(op: &LayeredOperation, run: &LayeredRun) -> Result<()> {
    ensure!(op.run_id == run.run_id, "foreign operation");
    ensure!(
        op.layer == run.layer || op.status.terminal(),
        "operation layer is ahead of the run"
    );
    Ok(())
}

fn ensure_event(event: &LayeredEvent, run: &LayeredRun) -> Result<()> {
    ensure!(event.run_id == run.run_id, "foreign layered event");
    ensure!(
        event
            .reason_summary
            .as_ref()
            .is_none_or(|s| s.chars().count() <= 1024)
            && event
                .decision_summary
                .as_ref()
                .is_none_or(|s| s.len() <= 128),
        "layered summaries exceed bounds"
    );
    Ok(())
}

/// The SQL upsert keyed by `execution_id`: an existing row with a different
/// `(run_id, operation_id)` silently swallows the write; `operation_id` is
/// globally unique, so a foreign owner is a constraint violation.
fn stage_operation(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    op: &LayeredOperation,
) -> Result<()> {
    let row_key = op_row_key(&op.execution_id);
    if let Some(existing) = kv::get_json::<LayeredOperation>(store, &row_key)? {
        if existing.run_id == op.run_id && existing.operation_id == op.operation_id {
            tx::put(batch, &row_key, op)?;
        }
        return Ok(());
    }
    if let Some(owner) = index::owner_str(store, &op_unique_index(&op.operation_id))? {
        if owner != op.execution_id {
            bail!("UNIQUE constraint failed: brain_layered_operations.operation_id");
        }
    }
    tx::put(batch, &row_key, op)?;
    index::put(
        batch,
        &op_unique_index(&op.operation_id),
        op.execution_id.as_bytes(),
    );
    index::put(
        batch,
        &op_index_key(&op.run_id, op.layer, &op.operation_id),
        op.execution_id.as_bytes(),
    );
    Ok(())
}

/// Append one event row plus its wake-receipt dedup marker. SQLite treats
/// NULLs as distinct inside unique indexes, so the marker only exists when
/// both `execution_id` and `source_sequence` are present.
fn stage_event(
    store: &rdb::store::Store,
    batch: &mut rocksdb::WriteBatch,
    staged_dedup: &mut HashSet<Vec<u8>>,
    event: &LayeredEvent,
) -> Result<()> {
    if let (Some(execution_id), Some(source_sequence)) =
        (&event.execution_id, event.source_sequence)
    {
        let dedup = event_dedup_key(&event.run_id, execution_id, source_sequence);
        let exists = staged_dedup.contains(&dedup)
            || rdb::store::get(store, b"", &dedup)
                .map_err(kv::engine_err)?
                .is_some();
        ensure!(
            !exists,
            "UNIQUE constraint failed: brain_layered_events.run_id, brain_layered_events.execution_id, brain_layered_events.source_sequence"
        );
        staged_dedup.insert(dedup.clone());
        tx::put_raw(batch, &dedup, &[]);
    }
    tx::put(batch, &event_key(&event.run_id, event.seq), event)
}

//! Host admission capacity: one live slot per execution, global FIFO by
//! sequence, completed tickets retained as durable history.
use super::super::keys;
use super::super::FleetStore;
use crate::rdb_store::{index, kv, seq, tx};
use anyhow::{ensure, Context, Result};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CapacitySnapshot {
    pub max_runs: u64,
    pub running: u64,
    pub queued: u64,
}

const SEQ_FAMILY: &str = "capq";
const TICKET_CONSTRAINT: &str = "capacity_queue.ticket";
const LIVE_CONSTRAINT: &str = "capacity_queue.execution_id";

fn stored_max_runs(store: &rdb::store::Store) -> Result<Option<i64>> {
    kv::get_json::<i64>(store, &keys::cap_key())
}

fn read_queue(store: &rdb::store::Store, sequence: i64) -> Result<Option<QueueRow>> {
    kv::get_json(store, &keys::cq_key(sequence))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct QueueRow {
    ticket: String,
    execution_id: String,
    runtime_id: String,
    phase: String,
}

pub(super) fn initialize_capacity(store: &rdb::store::Store, max_runs: usize) -> Result<()> {
    ensure!(
        max_runs > 0 && max_runs <= i64::MAX as usize,
        "invalid host capacity"
    );
    if stored_max_runs(store)?.is_none() {
        let mut batch = rocksdb::WriteBatch::default();
        batch.put(keys::cap_key(), kv::encode(&(max_runs as i64))?);
        tx::commit(store, batch)?;
    }
    Ok(())
}

pub(super) fn configure_capacity(store: &rdb::store::Store, max_runs: usize) -> Result<()> {
    ensure!(
        max_runs > 0 && max_runs <= i64::MAX as usize,
        "invalid host capacity"
    );
    let mut batch = rocksdb::WriteBatch::default();
    batch.put(keys::cap_key(), kv::encode(&(max_runs as i64))?);
    tx::commit(store, batch)
}

pub(super) fn capacity(store: &rdb::store::Store) -> Result<CapacitySnapshot> {
    let max_runs = stored_max_runs(store)?.context("host capacity is not configured")?;
    let (mut running, mut queued) = (0u64, 0u64);
    for (_, value) in tx::scan_prefix(store, &keys::cq_prefix())? {
        match kv::decode::<QueueRow>(&value)?.phase.as_str() {
            "running" => running += 1,
            "queued" => queued += 1,
            _ => {}
        }
    }
    Ok(CapacitySnapshot {
        max_runs: max_runs as u64,
        running,
        queued,
    })
}

/// Idempotent enqueue: schedulers replay accepted tickets on every tick, and
/// a replay returns the original sequence instead of allocating a new one.
pub(super) fn enqueue_capacity(
    store: &rdb::store::Store,
    ticket: &str,
    execution: &str,
    runtime: &str,
) -> Result<i64> {
    if let Some(sequence) = ticket_sequence(store, ticket, execution, runtime)? {
        return Ok(sequence);
    }
    let mut batch = rocksdb::WriteBatch::default();
    let sequence = seq::next_global(store, &mut batch, SEQ_FAMILY)?;
    let row_key = keys::cq_key(sequence);
    // The ticket index stores its sequence (read back via `owner_i64`) and
    // the partial `one_live_slot` index stores the row key; both are written
    // in the SAME batch as the row they constrain.
    if index::owner(store, &keys::cq_ticket_index(ticket))?.is_some() {
        anyhow::bail!("UNIQUE constraint failed: {TICKET_CONSTRAINT}");
    }
    index::put_i64(&mut batch, &keys::cq_ticket_index(ticket), sequence)?;
    index::claim_unique(
        store,
        &mut batch,
        &keys::cq_live_index(execution),
        &row_key,
        LIVE_CONSTRAINT,
    )?;
    tx::put(
        &mut batch,
        &row_key,
        &QueueRow {
            ticket: ticket.to_owned(),
            execution_id: execution.to_owned(),
            runtime_id: runtime.to_owned(),
            phase: "queued".into(),
        },
    )?;
    tx::commit(store, batch)?;
    Ok(sequence)
}

/// Caller holds the engine's write lease, so the read-then-insert pair is
/// race-free in-process; ownership is still checked on both reads.
fn ticket_sequence(
    store: &rdb::store::Store,
    ticket: &str,
    execution: &str,
    runtime: &str,
) -> Result<Option<i64>> {
    let Some(sequence) = index::owner_i64(store, &keys::cq_ticket_index(ticket))? else {
        return Ok(None);
    };
    let row = read_queue(store, sequence)?.context("capacity ticket disappeared after enqueue")?;
    ensure!(
        row.execution_id == execution && row.runtime_id == runtime && row.phase != "done",
        "capacity ticket conflict"
    );
    Ok(Some(sequence))
}

/// Claim the FIFO head while a live slot is free. Everything is decided under
/// the write lease, so the eligibility check and the phase flip are one step.
pub(super) fn claim_capacity(
    store: &rdb::store::Store,
    ticket: &str,
    runtime: &str,
) -> Result<bool> {
    let Some(sequence) = index::owner_i64(store, &keys::cq_ticket_index(ticket))? else {
        return Ok(false);
    };
    let Some(row) = read_queue(store, sequence)? else {
        return Ok(false);
    };
    if row.runtime_id != runtime || row.phase != "queued" {
        return Ok(false);
    }
    let Some(cap) = stored_max_runs(store)? else {
        return Ok(false);
    };
    // Global FIFO: the lowest-sequence live queued ticket claims first.
    let mut head = None;
    let mut running = 0i64;
    for (_, value) in tx::scan_prefix(store, &keys::cq_prefix())? {
        let row = kv::decode::<QueueRow>(&value)?;
        match row.phase.as_str() {
            "running" => running += 1,
            "queued" if head.is_none() => head = Some(row),
            _ => {}
        }
    }
    let Some(head) = head else {
        return Ok(false);
    };
    if head.ticket != ticket || running >= cap {
        return Ok(false);
    }
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(
        &mut batch,
        &keys::cq_key(sequence),
        &QueueRow {
            phase: "running".into(),
            ..row
        },
    )?;
    tx::commit(store, batch)?;
    Ok(true)
}

/// Only the owning runtime calls this after completion and durable writes.
/// No timer, host retirement, or release rollback is allowed to call it.
pub(super) fn finish_capacity(
    store: &rdb::store::Store,
    ticket: &str,
    runtime: &str,
) -> Result<()> {
    let sequence = ticket_or_bail(store, ticket, runtime)?;
    let row = read_queue(store, sequence)?.context("unknown capacity owner")?;
    if row.phase != "done" {
        retire(store, sequence, &row)?;
    }
    Ok(())
}

/// Release a running slot during restart recovery after the caller has
/// proved that the owning process is gone. The identity check runs under the
/// write lease, so a stale recovery receipt cannot release a ticket that was
/// reassigned or changed by another writer.
pub(super) fn recover_capacity(
    store: &rdb::store::Store,
    ticket: &str,
    execution: &str,
    runtime: &str,
) -> Result<bool> {
    let sequence = index::owner_i64(store, &keys::cq_ticket_index(ticket))?
        .with_context(|| format!("unknown capacity ticket {ticket}"))?;
    let row = read_queue(store, sequence)?
        .with_context(|| format!("unknown capacity ticket {ticket}"))?;
    ensure!(
        row.execution_id == execution && row.runtime_id == runtime,
        "capacity ticket identity differs"
    );
    if row.phase == "done" {
        return Ok(false);
    }
    ensure!(row.phase == "running", "capacity ticket is not running");
    retire(store, sequence, &row)?;
    Ok(true)
}

fn ticket_or_bail(store: &rdb::store::Store, ticket: &str, runtime: &str) -> Result<i64> {
    let sequence = index::owner_i64(store, &keys::cq_ticket_index(ticket))?
        .context("unknown capacity owner")?;
    let row = read_queue(store, sequence)?.context("unknown capacity owner")?;
    ensure!(row.runtime_id == runtime, "unknown capacity owner");
    Ok(sequence)
}

fn retire(store: &rdb::store::Store, sequence: i64, row: &QueueRow) -> Result<()> {
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(
        &mut batch,
        &keys::cq_key(sequence),
        &QueueRow {
            phase: "done".into(),
            ..(*row).clone()
        },
    )?;
    batch.delete(keys::cq_live_index(&row.execution_id));
    tx::commit(store, batch)
}

pub(super) fn runtime_tickets(
    store: &rdb::store::Store,
    runtime: &str,
) -> Result<Vec<(String, String, String)>> {
    let mut tickets = Vec::new();
    for (_, value) in tx::scan_prefix(store, &keys::cq_prefix())? {
        let row = kv::decode::<QueueRow>(&value)?;
        if row.runtime_id == runtime && row.phase != "done" {
            tickets.push((row.ticket, row.execution_id, row.phase));
        }
    }
    Ok(tickets) // scan order = sequence order
}

#[path = "access/capacity.rs"]
mod database_access;

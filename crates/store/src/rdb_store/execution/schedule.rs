//! Schedule persistence over the rdb KV engine: the DEFINITION family
//! (`sched/<id>`, the control-plane cron scheduler's source of truth) and
//! the fire ledger (`srun/<schedule_id>/<scheduled_for_ms:020>`). Ported
//! from the SQL backend's `execution/schedule`.
//!
//! The ledger key IS the SQL primary key `(schedule_id, scheduled_for_ms)`,
//! so re-firing the same tick replaces the row in place (error-retry
//! convergence) and no node registry is consulted (the ledger intentionally
//! has no FK: definitions outlive node churn).

use anyhow::Result;
use opencoder_core::config::ScheduleJob;
use serde::{Deserialize, Serialize};

use crate::rdb_store::kv::{self, pad_ord};
use crate::rdb_store::{seq, tx};
use crate::schedule_types::{ScheduleDefRecord, ScheduleRunRecord};

/// Persisted definition row: the job body plus server-owned timestamps.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SchedRow {
    job: ScheduleJob,
    created_at: i64,
    updated_at: i64,
}

fn def_key(id: &str) -> Vec<u8> {
    format!("sched/{id}").into_bytes()
}

fn run_prefix(schedule_id: &str) -> Vec<u8> {
    format!("srun/{schedule_id}/").into_bytes()
}

fn run_key(schedule_id: &str, scheduled_for_ms: i64) -> Vec<u8> {
    let mut k = run_prefix(schedule_id);
    k.extend_from_slice(pad_ord(scheduled_for_ms).as_bytes());
    k
}

// ── definitions ───────────────────────────────────────────────────────────

/// Insert or update one definition by id. `created_at` is stamped on the
/// first insert and PRESERVED on conflict (the API never lets a client
/// rewrite creation metadata); `updated_at` always moves to `now_ms`. No
/// validation here — callers validate before calling.
pub fn upsert_def(store: &rdb::store::Store, job: &ScheduleJob, now_ms: i64) -> Result<()> {
    let key = def_key(&job.id);
    let row = match kv::get_json::<SchedRow>(store, &key)? {
        Some(existing) => SchedRow {
            job: job.clone(),
            created_at: existing.created_at,
            updated_at: now_ms,
        },
        None => SchedRow {
            job: job.clone(),
            created_at: now_ms,
            updated_at: now_ms,
        },
    };
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &key, &row)?;
    tx::commit(store, batch)
}

pub fn get_def(store: &rdb::store::Store, id: &str) -> Result<Option<ScheduleDefRecord>> {
    Ok(kv::get_json::<SchedRow>(store, &def_key(id))?.map(record_of))
}

/// Every definition, stable `id` order (the list surface and the scheduler
/// scan iterate it; order must not depend on insert history).
pub fn list_defs(store: &rdb::store::Store) -> Result<Vec<ScheduleDefRecord>> {
    let mut rows = Vec::new();
    for (_, value) in tx::scan_prefix(store, b"sched/")? {
        rows.push(record_of(kv::decode::<SchedRow>(&value)?));
    }
    rows.sort_by(|a, b| a.job.id.cmp(&b.job.id));
    Ok(rows)
}

/// Delete one definition. Its fire ledger rows are KEPT: fire history is an
/// audit trail and outlives the definition.
pub fn delete_def(store: &rdb::store::Store, id: &str) -> Result<()> {
    let mut batch = rocksdb::WriteBatch::default();
    tx::del(&mut batch, &def_key(id));
    tx::commit(store, batch)
}

fn record_of(row: SchedRow) -> ScheduleDefRecord {
    ScheduleDefRecord {
        job: row.job,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

// ── fire ledger ───────────────────────────────────────────────────────────

/// Insert or replace one fire row (idempotent per tick: the deterministic
/// key makes a retried fire converge instead of duplicating).
pub fn record(store: &rdb::store::Store, rec: &ScheduleRunRecord) -> Result<()> {
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(
        &mut batch,
        &run_key(&rec.schedule_id, rec.scheduled_for_ms),
        rec,
    )?;
    tx::commit(store, batch)
}

/// The most recent row of `schedule_id` (latest tick wins), or `None`
/// before its first fire.
pub fn last(store: &rdb::store::Store, schedule_id: &str) -> Result<Option<ScheduleRunRecord>> {
    let prefix = run_prefix(schedule_id);
    let max = seq::max_child(store, &prefix);
    if max == 0 {
        return Ok(None);
    }
    kv::get_json(store, &run_key(schedule_id, max))
}

/// History of `schedule_id`, newest tick first, at most `limit` rows.
pub fn list(
    store: &rdb::store::Store,
    schedule_id: &str,
    limit: u32,
) -> Result<Vec<ScheduleRunRecord>> {
    let mut rows = Vec::new();
    for (_, value) in tx::scan_prefix(store, &run_prefix(schedule_id))? {
        rows.push(kv::decode::<ScheduleRunRecord>(&value)?);
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.scheduled_for_ms));
    rows.truncate(limit as usize);
    Ok(rows)
}

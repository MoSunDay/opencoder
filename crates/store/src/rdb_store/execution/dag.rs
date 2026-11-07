//! DAG workflow store (`dag_defs` / `dag_runs`) over the rdb KV engine —
//! definitions and the dispatched-run lifecycle: dispatch, FIFO claim,
//! transitions, cancel, lost-node convergence. Ported from the SQL
//! backend's `execution/dag` (claim / cancel-piggyback / lost-sweep reuse
//! the node_tasks protocols; the grid lives in `opencoder_dag::transitions`).
//! The append-only event stream lives in [`super::dag_events`].
//!
//! Key families:
//! - `dagdef/<id>`                     definition row (value = the record)
//! - `idx/dagdef-name/<name>` → row key   (UNIQUE name: upsert conflict key)
//! - `dagrun/<ord:020><run_id>`        run row; the zero-padded insertion
//!   ordinal is the `rowid` stand-in (same-ms FIFO tiebreak)
//! - `idx/dagrun-id/<run_id>` → row key   (PK lookup)

use anyhow::{bail, Result};
use opencoder_dag::{transition_allowed, DagRunStatus};

use crate::rdb_store::kv::{self, pad_ord};
use crate::rdb_store::{index, seq, tx};
use crate::types::{ConvergedDagRun, DagDefRecord, DagRunRecord};

include!("dag/parts/part_1.rs");

const RUN_ROW_FAMILY: &str = "dagrun-row";

fn def_key(id: &str) -> Vec<u8> {
    format!("dagdef/{id}").into_bytes()
}

fn def_name_index(name: &str) -> Vec<u8> {
    format!("idx/dagdef-name/{name}").into_bytes()
}

fn run_prefix() -> Vec<u8> {
    b"dagrun/".to_vec()
}

fn run_row_key(row_ord: i64, run_id: &str) -> Vec<u8> {
    let mut k = run_prefix();
    k.extend_from_slice(pad_ord(row_ord).as_bytes());
    k.extend_from_slice(run_id.as_bytes());
    k
}

/// Split `dagrun/<ord:020><run_id>` into (insertion ordinal, run id).
fn parse_run_key(key: &[u8]) -> Option<(i64, String)> {
    let text = std::str::from_utf8(key).ok()?;
    let leaf = text.strip_prefix("dagrun/")?;
    if leaf.len() <= kv::ORD_WIDTH {
        return None;
    }
    let (ord, run_id) = leaf.split_at(kv::ORD_WIDTH);
    Some((ord.parse().ok()?, run_id.to_string()))
}

fn run_id_index(run_id: &str) -> Vec<u8> {
    format!("idx/dagrun-id/{run_id}").into_bytes()
}

// ── definitions ───────────────────────────────────────────────────────────

/// Upsert keyed by `name` (the conflict target): a re-publish replaces
/// `spec_json`/`updated_at` while the FIRST row's `id`/`created_at` stay put.
pub fn upsert_def(store: &rdb::store::Store, def: &DagDefRecord) -> Result<()> {
    let name_index = def_name_index(&def.name);
    let mut batch = rocksdb::WriteBatch::default();
    match index::owner(store, &name_index)? {
        // Same name: refresh spec + updated_at on the ORIGINAL row.
        Some(row_key) => {
            let mut kept = kv::get_json::<DagDefRecord>(store, row_key.as_slice())?
                .ok_or_else(|| anyhow::anyhow!("upsert_dag_def: row vanished"))?;
            kept.spec_json = def.spec_json.clone();
            kept.updated_at = def.updated_at;
            tx::put(&mut batch, row_key.as_slice(), &kept)?;
        }
        None => {
            // Fresh name, but the id must not collide with another row's PK.
            let key = def_key(&def.id);
            if let Some(existing) = kv::get_json::<DagDefRecord>(store, &key)? {
                if existing.name != def.name {
                    bail!("UNIQUE constraint failed: dag_defs.id");
                }
            }
            index::claim_unique(store, &mut batch, &name_index, &key, "dag_defs.name")?;
            tx::put(&mut batch, &key, def)?;
        }
    }
    tx::commit(store, batch)
}

pub fn list_defs(store: &rdb::store::Store) -> Result<Vec<DagDefRecord>> {
    let mut out = Vec::new();
    for (_, value) in tx::scan_prefix(store, b"dagdef/")? {
        out.push(kv::decode::<DagDefRecord>(&value)?);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

pub fn get_def(store: &rdb::store::Store, id: &str) -> Result<Option<DagDefRecord>> {
    kv::get_json(store, &def_key(id))
}

pub fn delete_def(store: &rdb::store::Store, id: &str) -> Result<()> {
    let Some(row) = kv::get_json::<DagDefRecord>(store, &def_key(id))? else {
        return Ok(());
    };
    let mut batch = rocksdb::WriteBatch::default();
    tx::del(&mut batch, &def_key(id));
    tx::del(&mut batch, &def_name_index(&row.name));
    tx::commit(store, batch)
}

// ── runs: dispatch / claim ────────────────────────────────────────────────

/// Enqueue a run — always fresh: `status` is forced to `pending` and
/// `error`/`claimed_at`/`finished_at` cleared no matter what the caller's
/// record carries. No synthetic session: the `spec_json` snapshot carries
/// all execution context.
pub fn dispatch(store: &rdb::store::Store, run: &DagRunRecord) -> Result<DagRunRecord> {
    let mut batch = rocksdb::WriteBatch::default();
    let row_ord = seq::next_global(store, &mut batch, RUN_ROW_FAMILY)?;
    let row_key = run_row_key(row_ord, &run.id);
    index::claim_unique(
        store,
        &mut batch,
        &run_id_index(&run.id),
        &row_key,
        "dag_runs.id",
    )?;
    let fresh = DagRunRecord {
        id: run.id.clone(),
        dag_id: run.dag_id.clone(),
        name: run.name.clone(),
        spec_json: run.spec_json.clone(),
        node_id: run.node_id.clone(),
        status: DagRunStatus::Pending,
        error: None,
        created_at: run.created_at,
        claimed_at: None,
        finished_at: None,
    };
    tx::put(&mut batch, &row_key, &fresh)?;
    tx::commit(store, batch)?;
    Ok(fresh)
}

/// Every run row with its insertion ordinal, in `created_at ASC, ord ASC`
/// order — the claim FIFO order (the row key is reconstructible from the
/// ordinal + run id).
fn runs_fifo(store: &rdb::store::Store) -> Result<Vec<(i64, DagRunRecord)>> {
    let mut rows = Vec::new();
    for (key, value) in tx::scan_prefix(store, &run_prefix())? {
        let Some((row_ord, _)) = parse_run_key(&key) else {
            continue;
        };
        rows.push((row_ord, kv::decode::<DagRunRecord>(&value)?));
    }
    rows.sort_by(|a, b| (&a.1.created_at, &a.0).cmp(&(&b.1.created_at, &b.0)));
    Ok(rows)
}

/// Atomically claim the oldest pending run this node may take (pinned to it
/// or unpinned). `None` when the node already has a `running`/`cancelling`
/// run (single-active policy) or nothing is due. The scan and the
/// conditional write run under the exclusive write lease in one batch, so
/// racing claimers serialize like `BEGIN IMMEDIATE` + the SQL CAS and a
/// loser simply observes the winner's `running` row.
pub fn claim_next(
    store: &rdb::store::Store,
    node_id: &str,
    now_ms: i64,
) -> Result<Option<DagRunRecord>> {
    let rows = runs_fifo(store)?;
    // (a) Single active run per node — `running` AND `cancelling` both hold
    // the slot: a mid-cancel node must not start a second run.
    if rows.iter().any(|(_, r)| {
        r.node_id.as_deref() == Some(node_id)
            && matches!(r.status, DagRunStatus::Running | DagRunStatus::Cancelling)
    }) {
        return Ok(None);
    }
    // (b) Oldest eligible pending run (pinned-to-node or unpinned).
    let Some((row_ord, candidate)) = rows.into_iter().find(|(_, r)| {
        r.status == DagRunStatus::Pending && r.node_id.as_deref().is_none_or(|n| n == node_id)
    }) else {
        return Ok(None);
    };
    // (c) CAS pending -> running; an unpinned row gets `node_id` stamped.
    let mut claimed = candidate;
    if claimed.status != DagRunStatus::Pending {
        return Ok(None); // lost the race — treat as "nothing claimed"
    }
    claimed.status = DagRunStatus::Running;
    claimed.node_id = Some(node_id.to_string());
    claimed.claimed_at = Some(now_ms);
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &run_row_key(row_ord, &claimed.id), &claimed)?;
    tx::commit(store, batch)?;
    Ok(Some(claimed))
}

//! Node inventory report application with watermark fencing and first-sync
//! pending recovery.
use super::keys;
use super::FleetStore;
use crate::rdb_store::kv;
use crate::rdb_store::tx;
use anyhow::{bail, ensure, Result};
use opencoder_core::fleet::{valid_id, ExecutionIndex};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

mod rows;
mod snapshot;

#[derive(Debug, thiserror::Error)]
#[error("index report baseline changed: {0}; complete snapshot required")]
pub struct IndexReportBaselineChanged(pub &'static str);

#[derive(Serialize, Deserialize)]
struct Watermark {
    generation: String,
    sequence: u64,
}

/// Snapshot the pending IDs eligible for first-sync recovery.
pub(super) fn pending_ids(store: &rdb::store::Store, node_id: &str) -> Result<Vec<String>> {
    let mut ids = Vec::new();
    for (_, value) in tx::scan_prefix(store, b"xrow/")? {
        let record: ExecutionIndex = kv::decode(&value)?;
        if record.node_id == node_id
            && record.status == opencoder_core::fleet::ExecutionStatus::Pending
        {
            ids.push(record.id);
        }
    }
    ids.sort();
    Ok(ids)
}

pub(super) fn apply_index_report(
    store: &rdb::store::Store,
    node_id: &str,
    records: &[ExecutionIndex],
    pending_at_begin: Option<&[String]>,
) -> Result<Vec<ExecutionIndex>> {
    apply_index_report_fenced(store, node_id, records, pending_at_begin, None)
}

pub(super) fn apply_index_report_fenced(
    store: &rdb::store::Store,
    node_id: &str,
    records: &[ExecutionIndex],
    pending_at_begin: Option<&[String]>,
    watermark: Option<(&str, u64)>,
) -> Result<Vec<ExecutionIndex>> {
    apply_report(
        store,
        node_id,
        records,
        pending_at_begin,
        watermark,
        None,
        false,
    )
    .map(|rows| rows.unwrap_or_default())
}

/// Negotiated reports must either commit at the expected base or fail.
pub(super) fn apply_report(
    store: &rdb::store::Store,
    node_id: &str,
    records: &[ExecutionIndex],
    pending_at_begin: Option<&[String]>,
    watermark: Option<(&str, u64)>,
    committed_base: Option<Option<u64>>,
    rebuilt_snapshot: bool,
) -> Result<Option<Vec<ExecutionIndex>>> {
    validate_report(node_id, records)?;
    let mut batch = rocksdb::WriteBatch::default();
    if let Some((generation, sequence)) = watermark {
        ensure!(
            sequence <= i64::MAX as u64,
            "index report sequence exceeds storage range"
        );
        let previous: Option<Watermark> = kv::get_json(store, &keys::wm_key(node_id))?;
        if let Some(old) = previous.as_ref() {
            if rebuilt_snapshot && generation == old.generation && sequence <= old.sequence {
                rows::validate(store, records)?;
                return Ok(Some(Vec::new()));
            }
            if let Some(base) = committed_base {
                // A retired Host must retain its socket for outstanding RPC
                // replies. Skipping it is distinct from accepting a delta or
                // losing its base.
                if generation < old.generation.as_str() {
                    return Ok(None);
                }
                if generation == old.generation && sequence <= old.sequence {
                    return Err(IndexReportBaselineChanged("sequence superseded").into());
                }
                if let Some(base) = base {
                    if generation != old.generation || base != old.sequence {
                        return Err(IndexReportBaselineChanged("committed base differs").into());
                    }
                }
            } else if generation < old.generation.as_str()
                || (generation == old.generation && sequence <= old.sequence)
            {
                return Ok(None);
            }
        } else if matches!(committed_base, Some(Some(_))) {
            return Err(IndexReportBaselineChanged("committed base missing").into());
        }
        let _ = tx::put(
            &mut batch,
            &keys::wm_key(node_id),
            &Watermark {
                generation: generation.to_owned(),
                sequence,
            },
        );
    }
    let recovered = apply_report_tx(
        store,
        node_id,
        records,
        pending_at_begin.unwrap_or_default(),
        &mut batch,
    )?;
    tx::commit(store, batch)?;
    Ok(Some(recovered))
}

fn validate_report(node_id: &str, records: &[ExecutionIndex]) -> Result<()> {
    if !valid_id(node_id) {
        bail!("invalid index report node id");
    }
    let mut ids = HashSet::with_capacity(records.len());
    for record in records {
        if !valid_id(&record.id) || record.node_id != node_id {
            bail!("invalid index report ownership: {}", record.id);
        }
        if !ids.insert(record.id.as_str()) {
            bail!("duplicate execution in index report: {}", record.id);
        }
    }
    Ok(())
}

/// Stage the report rows and the pending-recovery transitions. Rows absent
/// from a complete inventory that were pending at begin and never dispatched
/// are marked errored so they cannot wedge admission forever.
fn apply_report_tx(
    store: &rdb::store::Store,
    node_id: &str,
    records: &[ExecutionIndex],
    pending_at_begin: &[String],
    batch: &mut rocksdb::WriteBatch,
) -> Result<Vec<ExecutionIndex>> {
    let present: HashSet<_> = records.iter().map(|record| record.id.as_str()).collect();
    rows::apply(store, records, batch)?;
    let mut recovered = Vec::new();
    let mut checked = HashSet::new();
    for id in pending_at_begin {
        if present.contains(id.as_str()) || !checked.insert(id.as_str()) {
            continue;
        }
        let Some(mut record) = keys::read_index(store, id)? else {
            continue;
        };
        if record.node_id != node_id
            || record.status != opencoder_core::fleet::ExecutionStatus::Pending
        {
            continue;
        }
        // A prepared outbox entry may still be in flight on another server.
        // Absence from an inventory is not evidence that it was rejected.
        if rdb::store::get(store, b"", &keys::asg_key(id))
            .map_err(kv::engine_err)?
            .is_some()
        {
            continue;
        }
        record.status = opencoder_core::fleet::ExecutionStatus::Error;
        batch.put(keys::xrow_key(id), kv::encode(&record)?);
        recovered.push(record);
    }
    Ok(recovered)
}

pub(super) fn apply_index_report_committed(
    store: &rdb::store::Store,
    node_id: &str,
    records: &[ExecutionIndex],
    pending_at_begin: Option<&[String]>,
    watermark: (&str, u64),
    base_sequence: Option<u64>,
) -> Result<Option<Vec<ExecutionIndex>>> {
    ensure!(
        base_sequence.is_none() || pending_at_begin.is_none(),
        "delta reports cannot recover missing pending executions"
    );
    apply_report(
        store,
        node_id,
        records,
        pending_at_begin,
        Some(watermark),
        Some(base_sequence),
        false,
    )
}

#[path = "access/report.rs"]
mod database_access;

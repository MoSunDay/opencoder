// Read the prepared receipt set before loading frozen assignment payloads.
use super::super::keys;
use super::super::FleetStore;
use super::Receipt;
use crate::rdb_store::{kv, tx};
use anyhow::{Context, Result};
use opencoder_core::fleet::{Assignment, ExecutionKind};

/// Caller serializes operations for this subject with request_lock.
pub(super) fn prepared_receipt_for(
    store: &rdb::store::Store,
    scope: &str,
    subject: &str,
) -> Result<Option<String>> {
    for (key, value) in tx::scan_prefix(store, &keys::rcp_prefix(scope))? {
        let receipt: super::Receipt = kv::decode(&value)?;
        if receipt.phase == "prepared"
            && receipt.payload.pointer("/subject").and_then(|v| v.as_str()) == Some(subject)
        {
            let id = keys::rcp_id(&key);
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Bounded recovery of durable operations outside execution admission.
pub(super) fn pending_receipts(
    store: &rdb::store::Store,
    scope: &str,
    after: &str,
    limit: u32,
) -> Result<Vec<(String, super::Receipt)>> {
    let mut values = Vec::new();
    for (key, value) in tx::scan_prefix(store, &keys::rcp_prefix(scope))? {
        if values.len() >= limit.clamp(1, 128) as usize {
            break;
        }
        if keys::rcp_id(&key).as_str() <= after {
            continue;
        }
        let receipt: super::Receipt = kv::decode(&value)?;
        if receipt.phase != "prepared" {
            continue;
        }
        values.push((keys::rcp_id(&key), receipt));
    }
    Ok(values)
}

/// A normal execution's receipt key is its ID. Projects use their current run
/// ID instead; their `prk/<run-key>` index resolves it back to the frozen
/// assignment. Candidates merge from both branches, then sort by id.
pub(super) fn pending_assignments(
    store: &rdb::store::Store,
    after: &str,
    limit: u32,
) -> Result<Vec<Assignment>> {
    let limit = limit.min(128) as usize;
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut candidates: Vec<(String, Assignment)> = Vec::new();
    for (key, value) in tx::scan_prefix(store, &keys::rcp_prefix("execution"))? {
        let receipt: super::Receipt = kv::decode(&value)
            .with_context(|| format!("decode receipt {}", String::from_utf8_lossy(&key)))?;
        if receipt.phase != "prepared" {
            continue;
        }
        let id = keys::rcp_id(&key);
        if let Some(raw) =
            rdb::store::get(store, b"", &keys::asg_key(&id)).map_err(kv::engine_err)?
        {
            let assignment: Assignment = serde_json::from_slice(&raw)?;
            if assignment.request.kind != ExecutionKind::Project {
                candidates.push((id, assignment));
                continue;
            }
        }
        let Some(owner) = rdb::store::get(store, b"", &keys::prk_key(&id))
            .map_err(kv::engine_err)?
            .and_then(|raw| String::from_utf8(raw).ok())
        else {
            continue;
        };
        let Some(raw) =
            rdb::store::get(store, b"", &keys::asg_key(&owner)).map_err(kv::engine_err)?
        else {
            continue;
        };
        candidates.push((owner, serde_json::from_slice(&raw)?));
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(candidates
        .into_iter()
        .filter(|(id, _)| id.as_str() > after)
        .take(limit)
        .map(|(_, assignment)| assignment)
        .collect())
}

#[path = "access/pending.rs"]
mod database_access;

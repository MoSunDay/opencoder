//! Dispatch receipts: first-writer durable proof of an accepted request,
//! plus the frozen assignment each receipt gates.
use super::super::keys;
use super::super::records;
use super::FleetStore;
use crate::rdb_store::{kv, tx};
use anyhow::{ensure, Result};
use opencoder_core::fleet::{Assignment, CreateExecution, ExecutionKind, RpcReply};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Project sessions retain one owner while each initial admission has its
/// own run ID.
pub fn dispatch_key(request: &CreateExecution) -> &str {
    if request.kind == ExecutionKind::Project {
        request.input["run_id"].as_str().unwrap_or(&request.id)
    } else {
        &request.id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub fingerprint: String,
    pub phase: String,
    pub payload: Value,
}

/// Dispatch-time naming snapshot lifted out of `execution_assignments` for
/// list rendering. The five-field execution index stays free of detail
/// fields (protocol locked); this is a read-only projection of the frozen
/// assignment: the placement target plus the definition name/spec name.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionNames {
    pub target: Option<String>,
    pub definition_name: Option<String>,
    pub spec_name: Option<String>,
    #[serde(default)]
    pub plan_name: Option<String>,
}

pub(super) fn receipt(store: &rdb::store::Store, scope: &str, id: &str) -> Result<Option<Receipt>> {
    kv::get_json(store, &keys::rcp_key(scope, id))
}

/// First writer owns the original intent. Returns false on content reuse.
pub(super) fn claim_request(
    store: &rdb::store::Store,
    scope: &str,
    id: &str,
    fingerprint: &str,
) -> Result<bool> {
    if let Some(existing) = kv::get_json::<Receipt>(store, &keys::rcp_key(scope, id))? {
        return Ok(existing.fingerprint == fingerprint);
    }
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(
        &mut batch,
        &keys::rcp_key(scope, id),
        &Receipt {
            fingerprint: fingerprint.to_owned(),
            phase: "claimed".into(),
            payload: Value::Null,
        },
    )?;
    tx::commit(store, batch)?;
    Ok(true)
}

/// Caller holds request_lock for this key. Updates cannot change intent.
pub(super) fn save_receipt(
    store: &rdb::store::Store,
    scope: &str,
    id: &str,
    receipt: &Receipt,
) -> Result<()> {
    let key = keys::rcp_key(scope, id);
    let existing = kv::get_json::<Receipt>(store, &key)?
        .ok_or_else(|| anyhow::anyhow!("request receipt ownership conflict"))?;
    ensure!(
        existing.fingerprint == receipt.fingerprint,
        "request receipt ownership conflict"
    );
    let mut batch = rocksdb::WriteBatch::default();
    tx::put(&mut batch, &key, receipt)?;
    tx::commit(store, batch)
}

pub(super) fn assignment(store: &rdb::store::Store, id: &str) -> Result<Option<Assignment>> {
    kv::get_json(store, &keys::asg_key(id))
}

/// Batch lookup of dispatch-time names for the execution list (no N+1 per
/// id); ids without a stored assignment are simply absent from the map.
pub(super) fn execution_names(
    store: &rdb::store::Store,
    ids: &[String],
) -> Result<HashMap<String, ExecutionNames>> {
    let mut out = HashMap::with_capacity(ids.len());
    for id in ids {
        let Some(raw) = rdb::store::get(store, b"", &keys::asg_key(id)).map_err(kv::engine_err)?
        else {
            continue;
        };
        let value: Value = serde_json::from_slice(&raw)?;
        out.insert(
            id.clone(),
            ExecutionNames {
                target: pointer_str(&value, "/request/target"),
                definition_name: pointer_str(&value, "/definition/name"),
                spec_name: pointer_str(&value, "/definition/spec/name"),
                plan_name: pointer_str(&value, "/request/input/layered_request/plan/title"),
            },
        );
    }
    Ok(out)
}

fn pointer_str(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Freeze ownership and input before the RPC. Conversation indexes are
/// published only after the node confirms durable acceptance.
///
/// Receipt + assignment (+ index row + project run-key index) stage in ONE
/// batch, so a failed prepare leaves nothing written.
pub(super) fn prepare_assignment(
    store: &rdb::store::Store,
    assignment: &Assignment,
    fingerprint: &str,
) -> Result<()> {
    let i = &assignment.index;
    ensure!(
        i.id == assignment.request.id && i.kind == assignment.request.kind,
        "assignment index must match request id and kind"
    );
    let mut batch = rocksdb::WriteBatch::default();
    if let Some(raw) = rdb::store::get(store, b"", &keys::asg_key(&i.id)).map_err(kv::engine_err)? {
        let old: Assignment = serde_json::from_slice(&raw)?;
        ensure!(
            old.index.node_id == i.node_id
                && old.index.kind == i.kind
                && old.index.created_at == i.created_at,
            "assignment ownership conflict"
        );
        if old.request == assignment.request {
            return Ok(());
        }
        ensure!(
            old.request.kind == ExecutionKind::Project
                && dispatch_key(&old.request) != dispatch_key(&assignment.request),
            "assignment conflict"
        );
        let rejected = receipt(store, "execution", dispatch_key(&old.request))?;
        ensure!(
            rejected.is_some_and(|row| row.phase == "rejected"),
            "previous project dispatch is unresolved or accepted"
        );
        if i.kind == ExecutionKind::Project {
            batch.delete(keys::prk_key(dispatch_key(&old.request)));
        }
    }
    if !matches!(i.kind, ExecutionKind::Agent | ExecutionKind::Operator) {
        super::acceptance::stage_insert(store, &mut batch, i, None)?;
    } else if records::index(store, &i.id)?.is_some() {
        // The index row of an agent/operator dispatch is published by node
        // acceptance; a conflicting pre-existing row still must be fenced.
        super::acceptance::validate_row(store, i)?;
    }
    tx::put(&mut batch, &keys::asg_key(&i.id), assignment)?;
    if i.kind == ExecutionKind::Project {
        batch.put(
            keys::prk_key(dispatch_key(&assignment.request)),
            i.id.as_bytes(),
        );
    }
    let key = keys::rcp_key("execution", dispatch_key(&assignment.request));
    let existing = kv::get_json::<Receipt>(store, &key)?
        .filter(|row| row.fingerprint == fingerprint)
        .ok_or_else(|| anyhow::anyhow!("missing dispatch receipt"))?;
    tx::put(
        &mut batch,
        &key,
        &Receipt {
            phase: "prepared".into(),
            ..existing
        },
    )?;
    tx::commit(store, batch)
}

pub(super) fn finish_dispatch(
    store: &rdb::store::Store,
    id: &str,
    fingerprint: &str,
    reply: &RpcReply,
) -> Result<()> {
    ensure!(
        (200..300).contains(&reply.status),
        "node acceptance required"
    );
    let accepted: opencoder_core::fleet::ExecutionIndex =
        serde_json::from_value(reply.body.clone())?;
    let mut batch = rocksdb::WriteBatch::default();
    super::acceptance::stage_insert(store, &mut batch, &accepted, Some(id))?;
    let key = keys::rcp_key("execution", id);
    let existing = kv::get_json::<Receipt>(store, &key)?
        .filter(|row| row.fingerprint == fingerprint)
        .ok_or_else(|| anyhow::anyhow!("request receipt ownership conflict"))?;
    tx::put(
        &mut batch,
        &key,
        &Receipt {
            phase: "accepted".into(),
            payload: serde_json::to_value(reply)?,
            ..existing
        },
    )?;
    tx::commit(store, batch)
}

#[path = "access/receipts.rs"]
mod database_access;

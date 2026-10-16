//! Select the wire contract once, from the root's frozen execution request.
use crate::AppState;
use anyhow::{Context, Result};
use opencoder_core::{brain::*, fleet::*};
use serde_json::Value;
use std::sync::Arc;

#[cfg(test)]
#[path = "io/tests.rs"]
mod tests;

pub(super) async fn admitted_version(state: &AppState, intent: &Value) -> Result<u32> {
    let Some(parent_id) = intent["parent"]["run_id"].as_str() else {
        return Ok(EXECUTION_IO_WRITER_VERSION);
    };
    let parent = state
        .fleet
        .assignment(parent_id)
        .await?
        .context("parent assignment missing")?;
    execution_io_version(&parent.request.input)
}

pub(super) fn mark(input: &mut Value, version: u32) {
    if version != 0 {
        input["execution_io_version"] = serde_json::json!(version);
    }
}

/// A crash can leave only the claim, before an assignment is prepared. Its
/// immutable hash, rather than the current Server default, selects the exact
/// root request. This also handles a v1 claim retried on the reader bridge.
pub(super) async fn preserve_claim(state: &AppState, request: &mut CreateExecution) -> Result<()> {
    let Some(receipt) = state.fleet.receipt("execution", &request.id).await? else {
        return Ok(());
    };
    if receipt.phase != "claimed" {
        return Ok(());
    }
    for version in 0..=EXECUTION_IO_READER_VERSION {
        let mut candidate = request.clone();
        candidate
            .input
            .as_object_mut()
            .context("root input must be an object")?
            .remove("execution_io_version");
        mark(&mut candidate.input, version);
        if opencoder_core::token_hash(&serde_json::to_string(&candidate)?) == receipt.fingerprint {
            *request = candidate;
            return Ok(());
        }
    }
    anyhow::bail!("claimed root request differs from the frozen execution fingerprint")
}

/// Replaying the persisted request also preserves resolved values, node and
/// fingerprint when a child accepted Create but its reply was lost.
pub(super) async fn replay(
    state: &Arc<AppState>,
    run_id: &str,
    operation_id: &str,
    execution: &ExecutionRef,
) -> Result<Option<RpcReply>> {
    let Some(frozen) = state.fleet.assignment(&execution.id).await? else {
        return Ok(None);
    };
    let parent = if execution.kind == ExecutionKind::Brain {
        &frozen.request.input["layered_intent"]["parent"]
    } else {
        &frozen.request.input["brain_layered"]
    };
    if frozen.request.kind != execution.kind
        || parent["run_id"] != run_id
        || parent["operation_id"] != operation_id
    {
        return Ok(Some(RpcReply::error(
            409,
            "frozen execution belongs to another operation",
        )));
    }
    let reply =
        crate::api::executions::submit_private(state, frozen.request, frozen.private_context).await;
    Ok(Some(
        if execution.kind == ExecutionKind::Brain && reply.status < 300 {
            RpcReply {
                status: 202,
                body: serde_json::json!({"schema_version":7,"run_id":execution.id,"execution":reply.body}),
            }
        } else {
            reply
        },
    ))
}

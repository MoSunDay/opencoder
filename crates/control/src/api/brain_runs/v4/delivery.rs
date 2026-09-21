//! Control-plane adapter for the node-owned v4 layered outbox.
//!
//! The node owns the layered projection and its generation fence. Control only
//! resolves catalog entries, creates ordinary child executions, and forwards
//! terminal/cancellation receipts back to that owner.
use super::{gateway, read, runtime};
use crate::{api::brain_runs::runs, AppState};
use anyhow::{ensure, Context, Result};
use opencoder_core::{brain::layered::*, brain::*, fleet::*};
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn deliver(
    state: &Arc<AppState>,
    node: &str,
    source: &ExecutionRef,
    action: &str,
    input: Value,
) -> Result<()> {
    ensure!(
        source.kind != ExecutionKind::System,
        "invalid layered source"
    );
    match action {
        "layered_wake" => wake(state, node, source, input).await,
        "layered_dispatch" => dispatch(state, node, source, input).await,
        "layered_cancel" => cancel(state, node, source, input).await,
        "layered_terminal" => terminal(state, node, source, input).await,
        _ => anyhow::bail!("unknown v4 layered outbox action {action}"),
    }
}

async fn snapshot(state: &Arc<AppState>, id: &str) -> Result<LayeredSnapshot> {
    read::snapshot(state, id)
        .await
        .map_err(|reply| anyhow::anyhow!("layered snapshot: {}", reply.body))
}

async fn wake(
    state: &Arc<AppState>,
    _node: &str,
    source: &ExecutionRef,
    input: Value,
) -> Result<()> {
    ensure!(
        source.kind == ExecutionKind::Brain,
        "layered wake must be a root"
    );
    let generation = input["generation"]
        .as_u64()
        .context("wake generation required")?;
    let _lock = state
        .fleet
        .request_lock("brain-control", &source.id)
        .await?;
    // A newer Ready generation may appear after this wake was handled. Only
    // acknowledge the source or the context actually admitted by this call;
    // reading the latest snapshot here could consume the next round's wake.
    let acknowledged = runtime::wake(state, &source.id)
        .await?
        .unwrap_or(generation);
    let reply = runs::call(
        state,
        &source.id,
        "layered_wake_ack",
        json!({"generation":acknowledged}),
    )
    .await;
    ensure!(
        reply.status < 300,
        "layered wake acknowledgement: {}",
        reply.body
    );
    Ok(())
}

async fn dispatch(
    state: &Arc<AppState>,
    _node: &str,
    source: &ExecutionRef,
    input: Value,
) -> Result<()> {
    ensure!(
        source.kind == ExecutionKind::Brain,
        "layered dispatch must be a root"
    );
    let _lock = state
        .fleet
        .request_lock("brain-control", &source.id)
        .await?;
    let operation: LayeredOperation = serde_json::from_value(input["operation"].clone())?;
    let assignment: LayeredAssignment = serde_json::from_value(input["assignment"].clone())?;
    let capability: BrainCapabilityDescriptor =
        serde_json::from_value(input["capability"].clone())?;
    ensure!(operation.run_id == source.id, "dispatch run mismatch");
    let authorized = runs::call(
        state,
        &source.id,
        "layered_authorize",
        json!({"operation":operation,"assignment":assignment,"capability":capability}),
    )
    .await;
    if authorized.status == 409 {
        // A paused run must retain the unacknowledged frame so resume can
        // dispatch it. A cancelled or already terminal operation can safely
        // retire the stale frame.
        let current = snapshot(state, &source.id).await?;
        let stale = current
            .operations
            .iter()
            .find(|op| op.operation_id == operation.operation_id)
            .is_none_or(|op| op.status != LayeredOperationStatus::Creating || op.cancel_requested)
            || current.run.phase.terminal();
        if !stale {
            return Ok(());
        }
        acknowledge_dispatch(state, source, &operation.operation_id).await?;
        return Ok(());
    }
    ensure!(
        authorized.status < 300,
        "layered authorization: {}",
        authorized.body
    );
    let current = snapshot(state, &source.id).await?;
    let operation = current
        .operations
        .iter()
        .find(|op| op.operation_id == operation.operation_id)
        .context("layered operation disappeared")?;
    let reply = gateway::dispatch(state, &current.run, operation, &assignment, &capability).await?;
    let receipt = runs::call(
        state,
        &source.id,
        "layered_receipt",
        json!({"operation_id":operation.operation_id,"reply":reply}),
    )
    .await;
    ensure!(
        receipt.status < 300,
        "layered dispatch receipt: {}",
        receipt.body
    );
    if receipt.body["retry"] == true {
        return Ok(());
    }
    acknowledge_dispatch(state, source, &operation.operation_id).await
}

async fn acknowledge_dispatch(
    state: &Arc<AppState>,
    source: &ExecutionRef,
    operation_id: &str,
) -> Result<()> {
    let reply = runs::call(
        state,
        &source.id,
        "layered_dispatch_ack",
        json!({"operation_id":operation_id}),
    )
    .await;
    ensure!(
        reply.status < 300,
        "layered dispatch acknowledgement: {}",
        reply.body
    );
    Ok(())
}

async fn cancel(
    state: &Arc<AppState>,
    _node: &str,
    source: &ExecutionRef,
    input: Value,
) -> Result<()> {
    ensure!(
        source.kind == ExecutionKind::Brain,
        "layered cancel must be a root"
    );
    let _lock = state
        .fleet
        .request_lock("brain-control", &source.id)
        .await?;
    let operation: LayeredOperation = serde_json::from_value(input)?;
    ensure!(operation.run_id == source.id, "cancel run mismatch");
    let current = snapshot(state, &source.id).await?;
    let current = current
        .operations
        .iter()
        .find(|op| op.operation_id == operation.operation_id)
        .context("layered operation missing")?;
    if !current.status.terminal() {
        if let Some(index) = state.fleet.index(&current.execution_id).await? {
            ensure!(index.kind == current.execution_kind, "child kind changed");
            let reply = state
                .hub
                .call(
                    &index.node_id,
                    NodeOperation::Command {
                        execution: index.execution_ref(),
                        command: ExecutionCommand {
                            action: "cancel".into(),
                            input: Value::Null,
                        },
                    },
                )
                .await;
            ensure!(reply.status < 300, "child cancellation: {}", reply.body);
        } else {
            // Admission never committed an index. Make that durable fact a
            // cancelled terminal operation so a cancelled run cannot wait.
            let notice = LayeredTerminalEvent {
                run_id: source.id.clone(),
                operation_id: current.operation_id.clone(),
                execution_kind: current.execution_kind,
                execution_id: current.execution_id.clone(),
                status: LayeredOperationStatus::Cancelled,
                source_sequence: 0,
            };
            let reply = runs::call(state, &source.id, "layered_terminal", json!(notice)).await;
            ensure!(
                reply.status < 300,
                "synthetic cancellation receipt: {}",
                reply.body
            );
        }
    }
    let reply = runs::call(state, &source.id, "layered_cancel_ack", json!(current)).await;
    ensure!(
        reply.status < 300,
        "layered cancel acknowledgement: {}",
        reply.body
    );
    Ok(())
}

/// Parent binding of a reporting child: a leaf carries `brain_layered`, a
/// nested run declares the same binding under `layered_request.parent`.
fn binding(child: &Assignment) -> &Value {
    let input = &child.request.input;
    if input["brain_layered"].is_object() {
        &input["brain_layered"]
    } else {
        &input["layered_request"]["parent"]
    }
}

/// A leaf child or nested run reports its terminal status. The exact
/// `LayeredTerminalEvent` is preferred; a frame that carries only the parent
/// identity is resolved against the reporting execution.
fn notice(source: &ExecutionRef, input: &Value) -> Result<LayeredTerminalEvent> {
    let run_id = input["run_id"]
        .as_str()
        .context("terminal run_id required")?;
    let operation_id = input["operation_id"]
        .as_str()
        .context("terminal operation_id required")?;
    let status: LayeredOperationStatus =
        serde_json::from_value(input["status"].clone()).context("terminal status required")?;
    ensure!(status.terminal(), "terminal status required");
    let execution_kind = match input.get("execution_kind") {
        Some(kind) => serde_json::from_value(kind.clone())?,
        None => source.kind,
    };
    let execution_id = input["execution_id"]
        .as_str()
        .unwrap_or(&source.id)
        .to_owned();
    ensure!(
        source
            == &ExecutionRef {
                id: execution_id.clone(),
                kind: execution_kind
            },
        "terminal source mismatch"
    );
    Ok(LayeredTerminalEvent {
        run_id: run_id.into(),
        operation_id: operation_id.into(),
        execution_kind,
        execution_id,
        status,
        source_sequence: input["source_sequence"].as_u64().unwrap_or(0),
    })
}

async fn terminal(
    state: &Arc<AppState>,
    node: &str,
    source: &ExecutionRef,
    input: Value,
) -> Result<()> {
    let notice = notice(source, &input)?;
    // Validate the child while serializing duplicate frames, then release its
    // lock before acquiring the parent lock. Cancellation takes the inverse
    // route (parent then child), so this scope prevents a cross-operation
    // deadlock during a terminal/cancel race.
    {
        let _child_lock = state
            .fleet
            .request_lock("brain-control", &notice.execution_id)
            .await?;
        let child = state
            .fleet
            .assignment(&notice.execution_id)
            .await?
            .context("terminal child missing")?;
        ensure!(
            child.index.kind == notice.execution_kind,
            "terminal child kind mismatch"
        );
        ensure!(child.index.node_id == node, "terminal child owner mismatch");
        let binding = binding(&child);
        ensure!(
            binding["run_id"] == notice.run_id,
            "terminal parent mismatch"
        );
        ensure!(
            binding["operation_id"] == notice.operation_id,
            "terminal operation mismatch"
        );
        if let Some(node_id) = input["node_id"].as_str() {
            ensure!(binding["node_id"] == node_id, "terminal node mismatch");
        }
        if let Some(layer) = input["layer"].as_u64() {
            ensure!(binding["layer"] == layer, "terminal layer mismatch");
        }
    }
    let _root_lock = state
        .fleet
        .request_lock("brain-control", &notice.run_id)
        .await?;
    let root = state
        .fleet
        .index(&notice.run_id)
        .await?
        .context("layered root missing")?;
    ensure!(
        root.kind == ExecutionKind::Brain,
        "layered parent kind mismatch"
    );
    let reply = runs::call(state, &notice.run_id, "layered_terminal", json!(notice)).await;
    ensure!(
        reply.status < 300,
        "layered terminal receipt: {}",
        reply.body
    );
    let ack = state
        .hub
        .call(
            node,
            NodeOperation::Brain {
                execution: source.clone(),
                action: "notice_ack".into(),
                input: json!({"sequence":notice.source_sequence}),
            },
        )
        .await;
    ensure!(
        ack.status < 300,
        "layered terminal acknowledgement: {}",
        ack.body
    );
    Ok(())
}

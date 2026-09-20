//! Control-plane adapter for the node-owned v3 scheduler outbox.
//!
//! The node owns the scheduler projection and its generation fence.  Control
//! only resolves catalog entries, creates ordinary child executions, and
//! forwards terminal/cancellation receipts back to that owner.
use super::{gateway, runtime};
use crate::{api::brain_runs::runs, AppState};
use anyhow::{ensure, Context, Result};
use opencoder_core::{brain::*, fleet::*};
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
        "invalid scheduler source"
    );
    match action {
        "scheduler_wake" => wake(state, node, source, input).await,
        "scheduler_dispatch" => dispatch(state, node, source, input).await,
        "scheduler_cancel" => cancel(state, node, source, input).await,
        "scheduler_terminal" => terminal(state, node, source, input).await,
        _ => anyhow::bail!("unknown v3 scheduler outbox action {action}"),
    }
}

async fn wake(
    state: &Arc<AppState>,
    _node: &str,
    source: &ExecutionRef,
    input: Value,
) -> Result<()> {
    ensure!(
        source.kind == ExecutionKind::Brain,
        "scheduler wake must be a root"
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
        "scheduler_wake_ack",
        json!({"generation":acknowledged}),
    )
    .await;
    ensure!(
        reply.status < 300,
        "scheduler wake acknowledgement: {}",
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
        "scheduler dispatch must be a root"
    );
    let _lock = state
        .fleet
        .request_lock("brain-control", &source.id)
        .await?;
    let operation: BrainOperation = serde_json::from_value(input["operation"].clone())?;
    let item: BrainDispatchItem = serde_json::from_value(input["item"].clone())?;
    let capability: BrainCapabilityDescriptor =
        serde_json::from_value(input["capability"].clone())?;
    ensure!(operation.run_id == source.id, "dispatch run mismatch");
    let authorized = runs::call(
        state,
        &source.id,
        "scheduler_authorize",
        json!({"operation":operation,"item":item,"capability":capability}),
    )
    .await;
    if authorized.status == 409 {
        // A paused run must retain the unacknowledged frame so resume can
        // dispatch it.  A cancelled or already terminal operation can safely
        // retire the stale frame.
        let snapshot = runs::call(state, &source.id, "snapshot", Value::Null).await;
        ensure!(
            snapshot.status < 300,
            "scheduler snapshot: {}",
            snapshot.body
        );
        let snapshot: BrainSchedulerSnapshot = serde_json::from_value(snapshot.body)?;
        let stale = snapshot
            .operations
            .iter()
            .find(|op| op.operation_id == operation.operation_id)
            .is_none_or(|op| op.status != BrainOperationStatus::Creating || op.cancel_requested)
            || snapshot.run.phase.terminal();
        if !stale {
            return Ok(());
        }
        acknowledge_dispatch(state, source, &operation.operation_id).await?;
        return Ok(());
    }
    ensure!(
        authorized.status < 300,
        "scheduler authorization: {}",
        authorized.body
    );
    let snapshot = runs::call(state, &source.id, "snapshot", Value::Null).await;
    ensure!(
        snapshot.status < 300,
        "scheduler snapshot: {}",
        snapshot.body
    );
    let snapshot: BrainSchedulerSnapshot = serde_json::from_value(snapshot.body)?;
    let operation = snapshot
        .operations
        .iter()
        .find(|op| op.operation_id == operation.operation_id)
        .context("scheduler operation disappeared")?;
    let reply = gateway::dispatch(state, &snapshot.run, operation, &item, &capability).await?;
    let receipt = runs::call(
        state,
        &source.id,
        "scheduler_receipt",
        json!({"operation_id":operation.operation_id,"reply":reply}),
    )
    .await;
    ensure!(
        receipt.status < 300,
        "scheduler dispatch receipt: {}",
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
        "scheduler_dispatch_ack",
        json!({"operation_id":operation_id}),
    )
    .await;
    ensure!(
        reply.status < 300,
        "scheduler dispatch acknowledgement: {}",
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
        "scheduler cancel must be a root"
    );
    let _lock = state
        .fleet
        .request_lock("brain-control", &source.id)
        .await?;
    let operation: BrainOperation = serde_json::from_value(input)?;
    ensure!(operation.run_id == source.id, "cancel run mismatch");
    let snapshot = runs::call(state, &source.id, "snapshot", Value::Null).await;
    ensure!(
        snapshot.status < 300,
        "scheduler snapshot: {}",
        snapshot.body
    );
    let snapshot: BrainSchedulerSnapshot = serde_json::from_value(snapshot.body)?;
    let current = snapshot
        .operations
        .iter()
        .find(|op| op.operation_id == operation.operation_id)
        .context("scheduler operation missing")?;
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
            // Admission never committed an index.  Make that durable fact a
            // cancelled terminal operation so a cancelled run cannot wait.
            let notice = BrainSchedulerTerminalEvent {
                run_id: source.id.clone(),
                operation_id: current.operation_id.clone(),
                execution_kind: current.execution_kind,
                execution_id: current.execution_id.clone(),
                status: BrainOperationStatus::Cancelled,
                source_sequence: 0,
            };
            let reply = runs::call(state, &source.id, "scheduler_terminal", json!(notice)).await;
            ensure!(
                reply.status < 300,
                "synthetic cancellation receipt: {}",
                reply.body
            );
        }
    }
    let reply = runs::call(state, &source.id, "scheduler_cancel_ack", json!(current)).await;
    ensure!(
        reply.status < 300,
        "scheduler cancel acknowledgement: {}",
        reply.body
    );
    Ok(())
}

async fn terminal(
    state: &Arc<AppState>,
    node: &str,
    source: &ExecutionRef,
    input: Value,
) -> Result<()> {
    let notice: BrainSchedulerTerminalEvent = serde_json::from_value(input)?;
    ensure!(
        source
            == &ExecutionRef {
                id: notice.execution_id.clone(),
                kind: notice.execution_kind
            },
        "terminal source mismatch"
    );
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
            .context("child assignment missing")?;
        ensure!(child.index.node_id == node, "terminal child owner mismatch");
        ensure!(
            child.request.input["brain_scheduler"]["run_id"] == notice.run_id,
            "terminal parent mismatch"
        );
        ensure!(
            child.request.input["brain_scheduler"]["operation_id"] == notice.operation_id,
            "terminal operation mismatch"
        );
    }
    let _root_lock = state
        .fleet
        .request_lock("brain-control", &notice.run_id)
        .await?;
    let root = state
        .fleet
        .index(&notice.run_id)
        .await?
        .context("scheduler root missing")?;
    ensure!(
        root.kind == ExecutionKind::Brain,
        "scheduler parent kind mismatch"
    );
    let reply = runs::call(state, &notice.run_id, "scheduler_terminal", json!(notice)).await;
    ensure!(
        reply.status < 300,
        "scheduler terminal receipt: {}",
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
        "scheduler terminal acknowledgement: {}",
        ack.body
    );
    Ok(())
}

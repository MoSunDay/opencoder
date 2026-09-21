//! Node-owned v4 outbox: wake, layer dispatch, cancellation and the settle of
//! terminal root state.
//!
//! Frames stay aggregate: a report never publishes an operation before the
//! finite decision that created it is durable in the journal.
use super::state;
use crate::{journal::Record, Worker};
use anyhow::{Context, Result};
use opencoder_core::{brain::layered::*, fleet::NodeFrame};
use serde_json::{json, Value};

pub async fn frames(worker: &Worker, record: &Record) -> Result<Vec<NodeFrame>> {
    let Some(snapshot) = worker
        .inner
        .state
        .store
        .brain_layered(&record.assignment.index.id)
        .await?
    else {
        return Ok(vec![]);
    };
    let mut frames = vec![];
    let frame = |action: &str, input: Value| NodeFrame::Brain {
        execution: record.assignment.index.execution_ref(),
        action: action.into(),
        input,
    };
    if matches!(
        snapshot.run.phase,
        LayeredPhase::Ready | LayeredPhase::Deciding
    ) && (record.annotations["layered_wake_ack"].is_null()
        || record.annotations["layered_wake_ack"]
            .as_u64()
            .is_some_and(|ack| ack < snapshot.run.generation))
    {
        frames.push(frame(
            "layered_wake",
            json!({"generation":snapshot.run.generation}),
        ));
    }
    if snapshot.run.phase == LayeredPhase::Waiting
        && snapshot
            .operations
            .iter()
            .any(|op| op.status == LayeredOperationStatus::Creating)
    {
        let intent: LayeredDispatchIntent =
            serde_json::from_value(record.annotations["layered_intent"].clone())
                .context("missing durable layered dispatch intent")?;
        for op in snapshot.operations.iter().filter(|op| {
            op.status == LayeredOperationStatus::Creating
                && !op.cancel_requested
                && record.annotations["layered_dispatch_acks"][&op.operation_id] != true
        }) {
            let assignment = intent
                .assignments
                .iter()
                .find(|assignment| assignment.node_id == op.node_id)
                .context("dispatch assignment missing")?;
            let capability = intent
                .capabilities
                .iter()
                .find(|capability| capability.capability_id == op.capability_id)
                .context("dispatch capability missing")?;
            frames.push(frame(
                "layered_dispatch",
                json!({"operation":op,"assignment":assignment,"capability":capability}),
            ));
        }
    }
    for op in snapshot
        .operations
        .iter()
        .filter(|op| op.cancel_requested && !op.status.terminal())
    {
        if record.annotations["layered_cancel_acks"][&op.operation_id] != true {
            frames.push(frame("layered_cancel", json!(op)));
        }
    }
    // Surface terminal root state even if a crash happened before journal
    // finalization; the settle is idempotent.
    state::settle(worker, &snapshot).await?;
    Ok(frames)
}

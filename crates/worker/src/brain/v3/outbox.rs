use super::state;
use crate::{journal::Record, Worker};
use anyhow::{Context, Result};
use opencoder_core::{brain::*, fleet::*};
use serde_json::{json, Value};

pub async fn frames(worker: &Worker, record: &Record) -> Result<Vec<NodeFrame>> {
    let Some(snapshot) = worker
        .inner
        .state
        .store
        .brain_scheduler(&record.assignment.index.id)
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
        BrainSchedulerPhase::Ready | BrainSchedulerPhase::Deciding
    ) && (record.annotations["scheduler_wake_ack"].is_null()
        || record.annotations["scheduler_wake_ack"]
            .as_u64()
            .is_some_and(|ack| ack < snapshot.run.generation))
    {
        frames.push(frame(
            "scheduler_wake",
            json!({"generation":snapshot.run.generation}),
        ));
    }
    if snapshot.run.phase == BrainSchedulerPhase::Waiting
        && snapshot
            .operations
            .iter()
            .any(|o| o.status == BrainOperationStatus::Creating)
    {
        let intent: BrainDispatchIntent =
            serde_json::from_value(record.annotations["scheduler_intent"].clone())
                .context("missing durable scheduler dispatch intent")?;
        for op in snapshot.operations.iter().filter(|o| {
            o.status == BrainOperationStatus::Creating
                && !o.cancel_requested
                && record.annotations["scheduler_dispatch_acks"][&o.operation_id] != true
        }) {
            let item = intent
                .items
                .iter()
                .find(|i| i.capability_id == op.capability_id)
                .context("dispatch item missing")?;
            let capability = intent
                .capabilities
                .iter()
                .find(|c| c.capability_id == op.capability_id)
                .context("dispatch capability missing")?;
            frames.push(frame(
                "scheduler_dispatch",
                json!({"operation":op,"item":item,"capability":capability}),
            ));
        }
    }
    for op in snapshot
        .operations
        .iter()
        .filter(|o| o.cancel_requested && !o.status.terminal())
    {
        if record.annotations["scheduler_cancel_acks"][&op.operation_id] != true {
            frames.push(frame("scheduler_cancel", json!(op)));
        }
    }
    // Surface terminal root state even if a crash happened before journal finalization.
    state::settle(worker, &snapshot).await?;
    Ok(frames)
}

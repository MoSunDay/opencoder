//! Terminal folding, retries and the layer barrier.
use super::{change, event, execution_id, operation_id};
use anyhow::{ensure, Context, Result};
use opencoder_core::brain::layered::*;

/// Latest attempt of a node; attempts are monotonic per node.
pub(crate) fn latest_attempt<'a>(
    ops: &'a [LayeredOperation],
    node_id: &str,
) -> Option<&'a LayeredOperation> {
    ops.iter()
        .filter(|op| op.node_id == node_id)
        .max_by_key(|op| op.attempt)
}

/// The whole completed layer has a successful latest attempt per node.
pub(crate) fn layers_complete(
    plan: &LayeredPlan,
    ops: &[LayeredOperation],
    layer: u32,
) -> Result<bool> {
    if layer == 0 {
        return Ok(ops.is_empty());
    }
    let levels = super::layers(plan)?;
    for index in 0..layer {
        let level = levels
            .get(index as usize)
            .context("layer is out of the plan")?;
        for node_id in level {
            let done = latest_attempt(ops, node_id).is_some_and(|op| op.status.successful());
            if !done {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

pub fn terminal(
    snapshot: &LayeredSnapshot,
    request: &LayeredRequest,
    notice: &LayeredTerminalEvent,
    now: i64,
) -> Result<Option<LayeredChange>> {
    ensure!(
        notice.run_id == snapshot.run.run_id && notice.status.terminal(),
        "invalid terminal event"
    );
    let old = snapshot
        .operations
        .iter()
        .find(|op| op.operation_id == notice.operation_id)
        .context("unknown operation")?;
    ensure!(
        old.execution_id == notice.execution_id && old.execution_kind == notice.execution_kind,
        "terminal execution identity mismatch"
    );
    if old
        .source_sequence
        .is_some_and(|seq| seq >= notice.source_sequence)
    {
        return Ok(None);
    }
    let mut update = change(snapshot, now);
    let mut e = event(&update.run, "operation_terminal", None);
    e.layer = old.layer;
    e.node_id = Some(old.node_id.clone());
    e.attempt = Some(old.attempt);
    e.capability_id = Some(old.capability_id.clone());
    e.execution_kind = Some(old.execution_kind);
    e.execution_id = Some(old.execution_id.clone());
    e.source_sequence = Some(notice.source_sequence);
    e.decision_summary = Some(format!("{:?}", notice.status).to_lowercase());
    let settle_cancelled = snapshot.run.phase.terminal()
        && notice.status == LayeredOperationStatus::Cancelled
        && old.cancel_requested
        && !old.status.terminal();
    if old.status.terminal()
        || (snapshot.run.phase.terminal() && !settle_cancelled)
        || old.layer != snapshot.run.layer
    {
        e.reason_summary = Some("late terminal event".into());
        update.events.push(e);
        return Ok(Some(update));
    }
    let attempt = old.attempt;
    let node_id = old.node_id.clone();
    let op = update
        .operations
        .iter_mut()
        .find(|op| op.operation_id == notice.operation_id)
        .unwrap();
    op.status = notice.status;
    op.source_sequence = Some(notice.source_sequence);
    update.events.push(e);
    if snapshot.run.phase.terminal() {
        return Ok(Some(update));
    }
    if !notice.status.successful() {
        let max_attempts = request
            .plan
            .node(&node_id)
            .map(|node| node.retry.max_attempts)
            .unwrap_or(1);
        if attempt < max_attempts && notice.status != LayeredOperationStatus::Cancelled {
            let retry = LayeredOperation {
                operation_id: operation_id(&snapshot.run.run_id, old.layer, &node_id, attempt + 1),
                run_id: snapshot.run.run_id.clone(),
                layer: old.layer,
                node_id: node_id.clone(),
                attempt: attempt + 1,
                capability_id: old.capability_id.clone(),
                execution_kind: old.execution_kind,
                execution_id: execution_id(
                    &snapshot.run.run_id,
                    old.layer,
                    &node_id,
                    attempt + 1,
                    old.execution_kind,
                ),
                status: LayeredOperationStatus::Creating,
                source_sequence: None,
                cancel_requested: false,
            };
            let mut scheduled = event(&update.run, "operation_retry_scheduled", None);
            scheduled.node_id = Some(node_id);
            scheduled.attempt = Some(attempt + 1);
            scheduled.capability_id = Some(retry.capability_id.clone());
            scheduled.execution_kind = Some(retry.execution_kind);
            scheduled.execution_id = Some(retry.execution_id.clone());
            update.events.push(scheduled);
            update.operations.push(retry);
            return Ok(Some(update));
        }
        update.run.phase = LayeredPhase::Failed;
        update.run.error = Some(format!("node {node_id} failed after {attempt} attempt(s)"));
        update
            .events
            .push(event(&update.run, "run_failed", update.run.error.clone()));
        cancel_pending(&mut update);
    } else if layers_complete(&request.plan, &update.operations, update.run.layer)? {
        update
            .events
            .push(event(&update.run, "layer_barrier_reached", None));
        if update.run.phase != LayeredPhase::Paused {
            update.run.phase = LayeredPhase::Ready;
        }
    }
    Ok(Some(update))
}

/// Fold a successful child admission into the projection before the terminal
/// notice arrives.
pub fn admit(snapshot: &LayeredSnapshot, operation_id: &str, now: i64) -> Result<LayeredChange> {
    let op = snapshot
        .operations
        .iter()
        .find(|op| op.operation_id == operation_id)
        .context("unknown operation")?;
    ensure!(
        op.status == LayeredOperationStatus::Creating,
        "operation is not creating"
    );
    let mut update = change(snapshot, now);
    let target = update
        .operations
        .iter_mut()
        .find(|op| op.operation_id == operation_id)
        .unwrap();
    target.status = LayeredOperationStatus::Running;
    let mut e = event(&update.run, "operation_admitted", None);
    e.layer = op.layer;
    e.node_id = Some(op.node_id.clone());
    e.attempt = Some(op.attempt);
    e.capability_id = Some(op.capability_id.clone());
    e.execution_kind = Some(op.execution_kind);
    e.execution_id = Some(op.execution_id.clone());
    update.events.push(e);
    Ok(update)
}

pub fn command(
    snapshot: &LayeredSnapshot,
    plan: &LayeredPlan,
    action: &str,
    now: i64,
) -> Result<LayeredChange> {
    ensure!(!snapshot.run.phase.terminal(), "run is terminal");
    let mut update = change(snapshot, now);
    match action {
        "pause" => update.run.phase = LayeredPhase::Paused,
        "resume" => {
            ensure!(
                snapshot.run.phase == LayeredPhase::Paused,
                "only paused runs can resume"
            );
            update.run.phase = if layers_complete(plan, &snapshot.operations, snapshot.run.layer)? {
                LayeredPhase::Ready
            } else {
                LayeredPhase::Waiting
            };
            update.run.error = None;
        }
        "cancel" => {
            update.run.phase = LayeredPhase::Cancelled;
            cancel_pending(&mut update);
        }
        _ => anyhow::bail!("supported commands: pause, resume, cancel"),
    }
    update.events.push(event(
        &update.run,
        match action {
            "pause" => "run_paused",
            "resume" => "run_resumed",
            _ => "run_cancelled",
        },
        None,
    ));
    Ok(update)
}

pub(crate) fn cancel_pending(update: &mut LayeredChange) {
    let mut events = vec![];
    for op in &mut update.operations {
        if !op.status.terminal() && !op.cancel_requested {
            op.cancel_requested = true;
            let mut e = event(&update.run, "cancel_requested", None);
            e.layer = op.layer;
            e.node_id = Some(op.node_id.clone());
            e.attempt = Some(op.attempt);
            e.execution_id = Some(op.execution_id.clone());
            e.execution_kind = Some(op.execution_kind);
            e.capability_id = Some(op.capability_id.clone());
            events.push(e);
        }
    }
    update.events.extend(events);
}

//! Terminal folding, retries and the layer barrier.
use super::{change, event};
use anyhow::{ensure, Context, Result};
use opencoder_core::brain::layered::*;

pub fn terminal(
    snapshot: &LayeredSnapshot,
    _request: &LayeredRequest,
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
    // Old activations and already terminal operations cannot invalidate a live
    // decision generation, even if a producer sends a newer receipt sequence.
    if old.status.terminal() || old.activation != snapshot.run.activation {
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
    let settle_cancelled =
        snapshot.run.phase.terminal() && old.cancel_requested && !old.status.terminal();
    if old.status.terminal()
        || (snapshot.run.phase.terminal() && !settle_cancelled)
        || old.activation != snapshot.run.activation
    {
        e.reason_summary = Some("late terminal event".into());
        update.events.push(e);
        return Ok(Some(update));
    }
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
    if barrier_parts(&update.run, &update.operations) {
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
    _plan: &LayeredPlan,
    action: &str,
    now: i64,
) -> Result<LayeredChange> {
    ensure!(!snapshot.run.phase.terminal(), "run is terminal");
    let mut update = change(snapshot, now);
    match action {
        "pause" => update.run.phase = LayeredPhase::Paused,
        "resume" => {
            ensure!(
                matches!(
                    snapshot.run.phase,
                    LayeredPhase::Paused | LayeredPhase::Blocked
                ),
                "only paused or blocked runs can resume"
            );
            update.run.phase = if barrier(snapshot) {
                LayeredPhase::Ready
            } else {
                LayeredPhase::Waiting
            };
            // Keep the rejection visible to the next decision. A valid decision clears it.
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

fn barrier_parts(run: &LayeredRun, ops: &[LayeredOperation]) -> bool {
    if run.activation == 0 {
        return ops.is_empty();
    }
    let current: Vec<_> = ops
        .iter()
        .filter(|op| op.activation == run.activation)
        .collect();
    !current.is_empty() && current.iter().all(|op| op.status.terminal())
}
pub fn barrier(snapshot: &LayeredSnapshot) -> bool {
    barrier_parts(&snapshot.run, &snapshot.operations)
}
pub fn current_successful(snapshot: &LayeredSnapshot) -> bool {
    barrier(snapshot)
        && snapshot
            .operations
            .iter()
            .filter(|op| op.activation == snapshot.run.activation)
            .all(|op| op.status.successful())
}

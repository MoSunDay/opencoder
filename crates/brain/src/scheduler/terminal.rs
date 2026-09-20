use super::{change, event};
use anyhow::{ensure, Context, Result};
use opencoder_core::brain::*;

pub fn terminal(
    snapshot: &BrainSchedulerSnapshot,
    notice: &BrainSchedulerTerminalEvent,
    now: i64,
) -> Result<Option<BrainSchedulerChange>> {
    ensure!(
        notice.run_id == snapshot.run.run_id && notice.status.terminal(),
        "invalid terminal event"
    );
    let old = snapshot
        .operations
        .iter()
        .find(|o| o.operation_id == notice.operation_id)
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
    e.round = old.round;
    e.capability_id = Some(old.capability_id.clone());
    e.execution_kind = Some(old.execution_kind);
    e.execution_id = Some(old.execution_id.clone());
    e.source_sequence = Some(notice.source_sequence);
    e.decision_summary = Some(format!("{:?}", notice.status).to_lowercase());
    // A cancellation request can race the child's admission.  In a terminal
    // run the late cancellation notice still has to settle that sibling;
    // successful/failed late notices remain index-only history.
    let settle_cancelled = snapshot.run.phase.terminal()
        && notice.status == BrainOperationStatus::Cancelled
        && old.cancel_requested
        && !old.status.terminal();
    if old.status.terminal()
        || (snapshot.run.phase.terminal() && !settle_cancelled)
        || old.round != snapshot.run.round
    {
        e.reason_summary = Some("late terminal event".into());
        update.events.push(e);
        return Ok(Some(update));
    }
    let op = update
        .operations
        .iter_mut()
        .find(|o| o.operation_id == notice.operation_id)
        .unwrap();
    op.status = notice.status;
    op.source_sequence = Some(notice.source_sequence);
    update.events.push(e);
    // A late cancellation settles the operation index only. The root's
    // terminal phase and original error are immutable.
    if snapshot.run.phase.terminal() {
        return Ok(Some(update));
    }
    if !notice.status.successful() {
        update.run.phase = BrainSchedulerPhase::Failed;
        update.run.error = Some("child execution failed".into());
        update
            .events
            .push(event(&update.run, "run_failed", update.run.error.clone()));
        cancel_siblings(&mut update);
    } else if update
        .operations
        .iter()
        .filter(|o| o.round == update.run.round)
        .all(|o| o.status.successful())
    {
        update
            .events
            .push(event(&update.run, "round_barrier_reached", None));
        if update.run.phase != BrainSchedulerPhase::Paused {
            update.run.phase = BrainSchedulerPhase::Ready;
        }
    }
    Ok(Some(update))
}

/// Fold a successful child admission into the projection before execution
/// terminal events begin arriving.
pub fn admit(
    snapshot: &BrainSchedulerSnapshot,
    operation_id: &str,
    now: i64,
) -> Result<BrainSchedulerChange> {
    let op = snapshot
        .operations
        .iter()
        .find(|op| op.operation_id == operation_id)
        .context("unknown operation")?;
    ensure!(
        op.status == BrainOperationStatus::Creating,
        "operation is not creating"
    );
    let mut update = change(snapshot, now);
    let current = update
        .operations
        .iter_mut()
        .find(|o| o.operation_id == operation_id)
        .unwrap();
    current.status = BrainOperationStatus::Running;
    let mut event = event(&update.run, "operation_admitted", None);
    event.capability_id = Some(current.capability_id.clone());
    event.execution_kind = Some(current.execution_kind);
    event.execution_id = Some(current.execution_id.clone());
    update.events.push(event);
    Ok(update)
}
pub fn command(
    snapshot: &BrainSchedulerSnapshot,
    action: &str,
    now: i64,
) -> Result<BrainSchedulerChange> {
    ensure!(!snapshot.run.phase.terminal(), "run is terminal");
    let mut update = change(snapshot, now);
    match action {
        "pause" => update.run.phase = BrainSchedulerPhase::Paused,
        "resume" => {
            ensure!(
                snapshot.run.phase == BrainSchedulerPhase::Paused,
                "only paused runs can resume"
            );
            update.run.phase = if snapshot.operations.iter().all(|o| o.status.successful()) {
                BrainSchedulerPhase::Ready
            } else {
                BrainSchedulerPhase::Waiting
            };
        }
        "cancel" => {
            update.run.phase = BrainSchedulerPhase::Cancelled;
            cancel_siblings(&mut update);
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
fn cancel_siblings(update: &mut BrainSchedulerChange) {
    let mut events = vec![];
    for op in &mut update.operations {
        if !op.status.terminal() && !op.cancel_requested {
            op.cancel_requested = true;
            let mut e = event(&update.run, "cancel_requested", None);
            e.execution_id = Some(op.execution_id.clone());
            e.execution_kind = Some(op.execution_kind);
            e.capability_id = Some(op.capability_id.clone());
            events.push(e);
        }
    }
    update.events.extend(events);
}

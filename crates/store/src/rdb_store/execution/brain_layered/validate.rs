//! Generation fencing and operation-immutability checks for layered
//! commits (the `ensure!` half of the SQL transaction, evaluated against
//! the committed projection under the write lease).

use anyhow::{ensure, Result};
use opencoder_core::brain::layered::{
    LayeredChange, LayeredOperation, LayeredOperationStatus, LayeredPhase, LayeredSnapshot,
};

/// Reject the change BEFORE anything is staged: expected-generation CAS,
/// next-generation arithmetic, terminal-phase freezing and per-operation
/// identity/terminal immutability (including the narrow Error→Done
/// recovery lane for stopped runs).
pub(super) fn check(old: &Option<LayeredSnapshot>, change: &LayeredChange) -> Result<()> {
    ensure!(
        old.as_ref().map(|s| s.run.generation) == change.expected_generation,
        "layered run generation conflict"
    );
    ensure!(
        change.run.generation == change.expected_generation.map_or(0, |v| v + 1),
        "invalid next generation"
    );
    if let Some(old) = old {
        ensure!(
            !old.run.phase.terminal() || change.run.phase == old.run.phase,
            "terminal run cannot change phase"
        );
        ensure!(
            old.operations.iter().all(|o| change
                .operations
                .iter()
                .any(|n| compatible(o, n, old, change))),
            "operation identity or terminal state changed"
        );
    }
    Ok(())
}

/// Identity fields are frozen; a terminal attempt may only keep its status,
/// except the single recovery lane: an Error attempt of the CURRENT
/// activation may fold to Done while the run stays stopped, accepting only
/// a strictly higher receipt source sequence.
fn compatible(
    old: &LayeredOperation,
    new: &LayeredOperation,
    old_snapshot: &LayeredSnapshot,
    change: &LayeredChange,
) -> bool {
    let identity = new.operation_id == old.operation_id
        && new.run_id == old.run_id
        && new.layer == old.layer
        && new.round == old.round
        && new.activation == old.activation
        && new.node_id == old.node_id
        && new.attempt == old.attempt
        && new.capability_id == old.capability_id
        && new.execution_kind == old.execution_kind
        && new.execution_id == old.execution_id;
    if !identity {
        return false;
    }
    if !old.status.terminal() || old.status == new.status {
        return true;
    }
    old.status == LayeredOperationStatus::Error
        && new.status == LayeredOperationStatus::Done
        && matches!(
            old_snapshot.run.phase,
            LayeredPhase::Paused | LayeredPhase::Blocked
        )
        && change.run.phase == old_snapshot.run.phase
        && change.run.activation == old_snapshot.run.activation
        && change.run.layer == old_snapshot.run.layer
        && change.run.round == old_snapshot.run.round
        && change.run.valid_layers == old_snapshot.run.valid_layers
        && old.activation == old_snapshot.run.activation
        && !old.cancel_requested
        && !new.cancel_requested
        && old
            .source_sequence
            .zip(new.source_sequence)
            .is_some_and(|(previous, current)| current > previous)
}

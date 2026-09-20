//! Pure round scheduling and finite model activation for protocol v3.
mod activation;
mod decision;
mod terminal;
mod validation;
pub use activation::{activate, PROMPT};
pub use decision::{decide, execution_id};
use opencoder_core::brain::*;
pub use terminal::admit;
pub use terminal::{command, terminal};
pub use validation::{prefilter, validate_plan, validate_request};

pub fn event(run: &BrainSchedulerRun, kind: &str, reason: Option<String>) -> BrainSchedulerEvent {
    BrainSchedulerEvent {
        seq: 0,
        run_id: run.run_id.clone(),
        round: run.round,
        event_type: kind.into(),
        capability_id: None,
        execution_kind: None,
        execution_id: None,
        decision_summary: None,
        reason_summary: reason.map(|s| s.chars().take(1024).collect()),
        source_sequence: None,
        evidence_execution_ids: vec![],
        at_ms: run.updated_at,
    }
}
pub fn initialize(
    id: &str,
    request: &BrainSchedulerRequest,
    now: i64,
) -> anyhow::Result<BrainSchedulerChange> {
    validate_request(request)?;
    anyhow::ensure!(
        opencoder_core::fleet::valid_id(id) && id.starts_with("brain-"),
        "invalid brain run ID"
    );
    let run = BrainSchedulerRun {
        run_id: id.into(),
        phase: BrainSchedulerPhase::Ready,
        round: 0,
        generation: 0,
        last_event_seq: 0,
        error: None,
        created_at: now,
        updated_at: now,
    };
    Ok(BrainSchedulerChange {
        expected_generation: None,
        events: vec![event(&run, "run_created", None)],
        run,
        operations: vec![],
    })
}
pub fn change(snapshot: &BrainSchedulerSnapshot, now: i64) -> BrainSchedulerChange {
    let mut run = snapshot.run.clone();
    run.generation += 1;
    run.updated_at = now;
    BrainSchedulerChange {
        expected_generation: Some(snapshot.run.generation),
        run,
        operations: snapshot.operations.clone(),
        events: vec![],
    }
}
pub fn block(snapshot: &BrainSchedulerSnapshot, reason: String, now: i64) -> BrainSchedulerChange {
    let mut update = change(snapshot, now);
    update.run.phase = BrainSchedulerPhase::Blocked;
    update.run.error = Some(reason.chars().take(1024).collect());
    update.events.push(event(
        &update.run,
        "decision_blocked",
        update.run.error.clone(),
    ));
    update
}

//! Pure layered scheduling for schema version 4: levels, bindings, one-layer
//! decisions, retries and the layer barrier. No IO lives here.
mod activation;
mod context;
mod decide;
mod levels;
mod prompt;
mod terminal;
mod validate;
pub use activation::activate;
pub use context::layer_context;
pub use decide::{decide, execution_id, operation_id};
pub use levels::{ancestors, layer_of, layers};
use opencoder_core::brain::layered::*;
pub use prompt::{instruction, PROMPT};
pub use terminal::{admit, command, terminal};
pub use validate::{validate_plan, validate_request};

pub fn event(run: &LayeredRun, kind: &str, reason: Option<String>) -> LayeredEvent {
    LayeredEvent {
        seq: 0,
        run_id: run.run_id.clone(),
        layer: run.layer,
        event_type: kind.into(),
        node_id: None,
        attempt: None,
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

pub fn initialize(id: &str, request: &LayeredRequest, now: i64) -> anyhow::Result<LayeredChange> {
    validate_request(request)?;
    anyhow::ensure!(
        opencoder_core::fleet::valid_id(id) && id.starts_with("brain-"),
        "invalid brain run ID"
    );
    let run = LayeredRun {
        run_id: id.into(),
        phase: LayeredPhase::Ready,
        layer: 0,
        generation: 0,
        last_event_seq: 0,
        error: None,
        summary: None,
        depth: request.depth,
        parent: request.parent.clone(),
        created_at: now,
        updated_at: now,
    };
    Ok(LayeredChange {
        expected_generation: None,
        events: vec![event(&run, "run_created", None)],
        run,
        operations: vec![],
    })
}

pub fn change(snapshot: &LayeredSnapshot, now: i64) -> LayeredChange {
    let mut run = snapshot.run.clone();
    run.generation += 1;
    run.updated_at = now;
    LayeredChange {
        expected_generation: Some(snapshot.run.generation),
        run,
        operations: snapshot.operations.clone(),
        events: vec![],
    }
}

pub fn block(snapshot: &LayeredSnapshot, reason: String, now: i64) -> LayeredChange {
    let mut update = change(snapshot, now);
    update.run.phase = LayeredPhase::Blocked;
    update.run.error = Some(reason.clone());
    update
        .events
        .push(event(&update.run, "decision_blocked", Some(reason)));
    update
}

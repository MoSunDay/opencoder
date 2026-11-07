//! Shared brain-v4 layered fixtures (run/event/change/operation builders)
//! reused by the layered and reporting suites.

use opencoder_core::brain::layered::*;
use opencoder_core::fleet::ExecutionKind;
use opencoder_store::Store;

pub(crate) fn run(phase: LayeredPhase, layer: u32, generation: u64) -> LayeredRun {
    LayeredRun {
        pending_guidance: false,
        round: 1,
        activation: 0,
        valid_layers: 0,
        max_rounds: 5,
        reflection: None,
        run_id: "brain-layered-store".into(),
        phase,
        layer,
        generation,
        last_event_seq: 0,
        error: None,
        summary: None,
        depth: 0,
        parent: None,
        created_at: 1,
        updated_at: 1,
    }
}

pub(crate) fn event(event_type: &str, run: &LayeredRun) -> LayeredEvent {
    LayeredEvent {
        user_input: None,
        guidance: vec![],
        reflection: None,
        assessments: Default::default(),
        assignments: vec![],
        round: 1,
        activation: 0,
        seq: 0,
        run_id: run.run_id.clone(),
        layer: run.layer,
        event_type: event_type.into(),
        node_id: None,
        attempt: None,
        capability_id: None,
        execution_kind: None,
        execution_id: None,
        decision_summary: None,
        reason_summary: None,
        source_sequence: None,
        evidence_execution_ids: vec![],
        at_ms: 1,
    }
}

pub(crate) fn created() -> LayeredChange {
    let run = run(LayeredPhase::Ready, 0, 0);
    LayeredChange {
        expected_generation: None,
        events: vec![event("run_created", &run)],
        run,
        operations: vec![],
    }
}

pub(crate) fn operation(
    attempt: u32,
    status: LayeredOperationStatus,
    sequence: Option<u64>,
) -> LayeredOperation {
    LayeredOperation {
        round: 1,
        activation: 0,
        operation_id: format!("brain-layered-store#l1#impact#a{attempt}"),
        run_id: "brain-layered-store".into(),
        layer: 1,
        node_id: "impact".into(),
        attempt,
        capability_id: "agent-impact".into(),
        execution_kind: ExecutionKind::Agent,
        execution_id: format!("agent-a{attempt}"),
        status,
        source_sequence: sequence,
        cancel_requested: false,
    }
}

pub(crate) fn recovery_change(old: &LayeredSnapshot) -> LayeredChange {
    let mut change = LayeredChange {
        expected_generation: Some(old.run.generation),
        run: old.run.clone(),
        operations: old.operations.clone(),
        events: vec![event("operation_terminal", &old.run)],
    };
    change.run.generation += 1;
    change.operations[0].status = LayeredOperationStatus::Done;
    change.operations[0].source_sequence = Some(6);
    change.events[0].execution_id = Some(change.operations[0].execution_id.clone());
    change.events[0].source_sequence = Some(6);
    change
}

/// An existing run whose sole attempt already errored.
pub(crate) async fn stored_error(
    phase: LayeredPhase,
    activation: u64,
    cancelled: bool,
) -> (opencoder_store::rdb_store::RdbStore, LayeredSnapshot) {
    let store = super::mem().await;
    let mut initial = created();
    initial.run.phase = phase;
    initial.run.layer = 1;
    initial.run.activation = 4;
    let mut op = operation(1, LayeredOperationStatus::Error, Some(4));
    op.activation = activation;
    op.cancel_requested = cancelled;
    initial.operations.push(op);
    let snapshot = store.commit_brain_layered(&initial).await.unwrap();
    (store, snapshot)
}

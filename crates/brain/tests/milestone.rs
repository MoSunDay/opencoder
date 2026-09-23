//! The actual reflection/barrier state machine, without model or timing mocks.
use opencoder_brain::layered::*;
use opencoder_core::brain::{layered::*, BrainCapabilityDescriptor};
use serde_json::json;
fn request() -> LayeredRequest {
    serde_json::from_value(json!({"schema_version":6,"plan":{"schema_version":6,"title":"delivery","objective":"ship verified change","max_rounds":5,
      "nodes":[{"node_id":"code","layer":1,"title":"Coding","objective":"implement","success_criteria":"change works","capability_ids":["agent","review"]},
               {"node_id":"docs","layer":1,"title":"Docs","objective":"document","success_criteria":"accurate","capability_ids":["agent"]},
               {"node_id":"test","layer":2,"title":"Test","objective":"verify","success_criteria":"tests pass","capability_ids":["agent"]}],
      "edges":[]}})).unwrap()
}
fn catalog() -> Vec<BrainCapabilityDescriptor> {
    ["agent","review"].iter().map(|id| serde_json::from_value(json!({"capability_id":id,"kind":"agent","target":"act","input_desc":"task","output_desc":"result","definition":{},"version":"1"})).unwrap()).collect()
}
fn snap(change: LayeredChange) -> LayeredSnapshot {
    LayeredSnapshot {
        schema_version: 6,
        run: change.run,
        operations: change.operations,
    }
}
fn proposal(current: &LayeredSnapshot, req: &LayeredRequest, layer: u32) -> LayeredDecision {
    let assessments: serde_json::Map<_,_> = req.plan.nodes.iter().filter(|n| n.layer == current.run.layer).map(|n| (n.node_id.clone(),json!({"met":current.operations.iter().filter(|o| o.activation==current.run.activation && o.node_id==n.node_id).all(|o| o.status.successful()),"reason":"verified actual outputs"}))).collect();
    let assignments: Vec<_> = req.plan.nodes.iter().filter(|n| n.layer == layer).flat_map(|n| n.capability_ids.iter().map(|cap| json!({"node_id":n.node_id,"capability_id":cap,"inputs":{},"reason":"use attached capability"}))).collect();
    let decision: LayeredDecision = serde_json::from_value(json!({"decision":"dispatch_layer","layer":layer,"assignments":assignments,"assessments":assessments,"reason":"evaluate milestone evidence","reflection":if layer <= current.run.layer {Some("fix issues with new context")} else {None},"evidence_execution_ids":[]})).unwrap();
    decision
}
fn dispatch(current: &LayeredSnapshot, req: &LayeredRequest, layer: u32) -> LayeredChange {
    let mut deciding = current.clone();
    deciding.run.phase = LayeredPhase::Deciding;
    decide(
        &deciding,
        req,
        &catalog(),
        &proposal(current, req, layer),
        10,
    )
    .unwrap()
}
fn notice(op: &LayeredOperation, status: LayeredOperationStatus) -> LayeredTerminalEvent {
    LayeredTerminalEvent {
        run_id: op.run_id.clone(),
        operation_id: op.operation_id.clone(),
        execution_kind: op.execution_kind,
        execution_id: op.execution_id.clone(),
        status,
        source_sequence: 1,
    }
}
fn finish(
    mut current: LayeredSnapshot,
    req: &LayeredRequest,
    status: LayeredOperationStatus,
) -> LayeredSnapshot {
    let pending: Vec<_> = current
        .operations
        .iter()
        .filter(|op| op.activation == current.run.activation)
        .cloned()
        .collect();
    for op in pending {
        current = snap(
            terminal(&current, req, &notice(&op, status), 20)
                .unwrap()
                .unwrap(),
        );
    }
    current
}
#[test]
fn parallel_failures_wait_for_the_entire_frozen_dispatch_and_do_not_retry() {
    let req = request();
    let initial = snap(initialize("brain-method", &req, 1).unwrap());
    let current = snap(dispatch(&initial, &req, 1));
    assert_eq!(current.operations.len(), 3);
    let failed = notice(&current.operations[0], LayeredOperationStatus::Error);
    let partial = snap(terminal(&current, &req, &failed, 11).unwrap().unwrap());
    assert_eq!(partial.run.phase, LayeredPhase::Waiting);
    assert_eq!(partial.operations.len(), 3);
    assert!(terminal(&partial, &req, &failed, 12).unwrap().is_none());
    let mut final_state = partial;
    for op in current.operations.iter().skip(1) {
        final_state = snap(
            terminal(
                &final_state,
                &req,
                &notice(op, LayeredOperationStatus::Done),
                13,
            )
            .unwrap()
            .unwrap(),
        );
    }
    assert_eq!(final_state.run.phase, LayeredPhase::Ready);
    assert!(final_state.operations.iter().all(|op| !op.cancel_requested));
}
#[test]
fn return_starts_a_new_round_with_distinct_ids_and_old_receipts_cannot_advance_it() {
    let req = request();
    let initial = snap(initialize("brain-loop", &req, 1).unwrap());
    let first = finish(
        snap(dispatch(&initial, &req, 1)),
        &req,
        LayeredOperationStatus::Done,
    );
    let tested = finish(
        snap(dispatch(&first, &req, 2)),
        &req,
        LayeredOperationStatus::Error,
    );
    let returned = snap(dispatch(&tested, &req, 1));
    assert_eq!(returned.run.round, 2);
    assert_eq!(returned.run.valid_layers, 0);
    assert_eq!(returned.run.activation, 3);
    assert!(returned.run.reflection.is_some());
    let ids: std::collections::BTreeSet<_> = returned
        .operations
        .iter()
        .map(|o| &o.execution_id)
        .collect();
    assert_eq!(ids.len(), returned.operations.len());
    let stale = notice(&first.operations[0], LayeredOperationStatus::Done);
    assert!(terminal(&returned, &req, &stale, 50).unwrap().is_none());
}
#[test]
fn fifth_round_blocks_rework_without_creating_executions() {
    let req = request();
    let mut current = snap(initialize("brain-budget", &req, 1).unwrap());
    for round in 1..=5 {
        current = finish(
            snap(dispatch(&current, &req, 1)),
            &req,
            LayeredOperationStatus::Error,
        );
        assert_eq!(current.run.round, round);
    }
    let blocked = dispatch(&current, &req, 1);
    assert_eq!(blocked.run.phase, LayeredPhase::Blocked);
    assert_eq!(blocked.operations.len(), current.operations.len());
    let mut blocked = snap(blocked);
    blocked.run.max_rounds = 6;
    let resumed = snap(command(&blocked, &req.plan, "resume", 90).unwrap());
    let next = dispatch(&resumed, &req, 1);
    assert_eq!(next.run.round, 6);
    assert_eq!(next.run.phase, LayeredPhase::Waiting);
}
#[test]
fn only_all_successful_layers_can_complete_and_cycles_do_not_affect_grouping() {
    let req = request();
    assert_eq!(
        layers(&req.plan).unwrap(),
        vec![vec!["code", "docs"], vec!["test"]]
    );
    let initial = snap(initialize("brain-complete", &req, 1).unwrap());
    let mut first = finish(
        snap(dispatch(&initial, &req, 1)),
        &req,
        LayeredOperationStatus::Done,
    );
    first.run.phase = LayeredPhase::Deciding;
    let complete = LayeredDecision::Complete {
        assessments: [(
            "test".into(),
            MilestoneAssessment {
                met: true,
                reason: "tests pass".into(),
            },
        )]
        .into(),
        reason: "criteria passed".into(),
        evidence_execution_ids: vec![],
        summary: "delivered".into(),
    };
    assert!(decide(&first, &req, &catalog(), &complete, 50).is_err());
    let mut last = finish(
        snap(dispatch(&first, &req, 2)),
        &req,
        LayeredOperationStatus::Done,
    );
    last.run.phase = LayeredPhase::Deciding;
    assert_eq!(
        decide(&last, &req, &catalog(), &complete, 60)
            .unwrap()
            .run
            .phase,
        LayeredPhase::Completed
    );
}

#[path = "milestone/validation.rs"]
mod validation;

#[test]
fn resumed_decision_receives_exact_assessment_keys_and_previous_rejection() {
    let req = request();
    let first = snap(initialize("brain-feedback", &req, 1).unwrap());
    let first = snap(dispatch(&first, &req, 1));
    let first = finish(first, &req, LayeredOperationStatus::Done);
    let last = snap(dispatch(&first, &req, 2));
    let last = finish(last, &req, LayeredOperationStatus::Done);
    let rejected = snap(block(
        &last,
        "assess every current milestone exactly once".into(),
        20,
    ));
    let resumed = snap(command(&rejected, &req.plan, "resume", 21).unwrap());
    let mut admitted = resumed;
    admitted.run.phase = LayeredPhase::Deciding;
    let context = layer_context(&admitted, &req, &catalog(), Default::default(), None).unwrap();
    let prompt: serde_json::Value = serde_json::from_str(&instruction(&context).unwrap()).unwrap();
    assert_eq!(prompt["assessment_node_ids"], json!(["test"]));
    assert_eq!(
        prompt["run"]["error"],
        "assess every current milestone exactly once"
    );
    let decision = serde_json::from_value(json!({"decision":"complete","reason":"verified",
        "summary":"all done","assessments":{"test":{"met":true,"reason":"tests passed"}}}))
    .unwrap();
    admitted.run.phase = LayeredPhase::Deciding;
    let completed = decide(&admitted, &req, &catalog(), &decision, 23).unwrap();
    assert_eq!(completed.run.phase, LayeredPhase::Completed);
    assert!(completed.run.error.is_none());
}

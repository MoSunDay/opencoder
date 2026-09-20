use opencoder_brain::scheduler;
use opencoder_core::brain::*;
use opencoder_core::fleet::ExecutionKind;
use serde_json::json;
use std::collections::BTreeMap;

fn request() -> BrainSchedulerRequest {
    BrainSchedulerRequest {
        schema_version: 3,
        objective: "test".into(),
        inputs: [("repo".into(), json!("r"))].into(),
        max_rounds: 4,
        capability_ids: vec![],
        artifacts: BTreeMap::new(),
    }
}
fn cap(id: &str) -> BrainCapabilityDescriptor {
    BrainCapabilityDescriptor {
        capability_id: id.into(),
        kind: ExecutionKind::Agent,
        target: "act".into(),
        input_desc: "repo".into(),
        output_desc: "result".into(),
        required_inputs: vec!["repo".into()],
        definition: json!({"name":"act"}),
        version: "v1".into(),
    }
}
fn snapshot() -> BrainSchedulerSnapshot {
    BrainSchedulerSnapshot {
        schema_version: 3,
        run: BrainSchedulerRun {
            run_id: "brain-test".into(),
            phase: BrainSchedulerPhase::Deciding,
            round: 0,
            generation: 1,
            last_event_seq: 0,
            error: None,
            created_at: 1,
            updated_at: 1,
        },
        operations: vec![],
    }
}
#[test]
fn invalid_capability_and_missing_root_are_blocked() {
    let d = BrainSchedulerDecision::Dispatch {
        capabilities: vec![BrainDispatchItem {
            capability_id: "missing".into(),
            inputs: BTreeMap::new(),
        }],
        reason: "x".into(),
        evidence_execution_ids: vec![],
    };
    assert!(scheduler::decide(&snapshot(), &request(), &[cap("a")], &d, 2).is_err());
    let d = BrainSchedulerDecision::Dispatch {
        capabilities: vec![BrainDispatchItem {
            capability_id: "a".into(),
            inputs: BTreeMap::new(),
        }],
        reason: "x".into(),
        evidence_execution_ids: vec![],
    };
    assert!(scheduler::decide(&snapshot(), &request(), &[cap("a")], &d, 2).is_err());
}
#[test]
fn dispatch_and_complete_require_terminal_evidence() {
    let mut bindings = BTreeMap::new();
    bindings.insert(
        "repo".into(),
        BrainInputBinding::Root {
            name: "repo".into(),
        },
    );
    let dispatch = BrainSchedulerDecision::Dispatch {
        capabilities: vec![BrainDispatchItem {
            capability_id: "a".into(),
            inputs: bindings,
        }],
        reason: "analyze".into(),
        evidence_execution_ids: vec![],
    };
    let change = scheduler::decide(&snapshot(), &request(), &[cap("a")], &dispatch, 2).unwrap();
    assert_eq!(change.run.round, 1);
    assert!(change.operations[0].execution_id.starts_with("agent-"));
    let mut completed = snapshot();
    completed.run.round = 1;
    completed.operations = change
        .operations
        .into_iter()
        .map(|mut o| {
            o.status = BrainOperationStatus::Done;
            o
        })
        .collect();
    let decision = BrainSchedulerDecision::Complete {
        reason: "verified".into(),
        evidence_execution_ids: vec![completed.operations[0].execution_id.clone()],
    };
    assert_eq!(
        scheduler::decide(&completed, &request(), &[cap("a")], &decision, 3)
            .unwrap()
            .run
            .phase,
        BrainSchedulerPhase::Completed
    );
}
#[test]
fn failure_marks_run_and_requests_sibling_cancellation() {
    let mut s = snapshot();
    s.run.round = 1;
    s.operations = vec![
        BrainOperation {
            operation_id: "a".into(),
            run_id: s.run.run_id.clone(),
            round: 1,
            capability_id: "a".into(),
            execution_kind: ExecutionKind::Agent,
            execution_id: "agent-a".into(),
            status: BrainOperationStatus::Running,
            source_sequence: None,
            cancel_requested: false,
        },
        BrainOperation {
            operation_id: "b".into(),
            run_id: s.run.run_id.clone(),
            round: 1,
            capability_id: "b".into(),
            execution_kind: ExecutionKind::Agent,
            execution_id: "agent-b".into(),
            status: BrainOperationStatus::Running,
            source_sequence: None,
            cancel_requested: false,
        },
    ];
    let n = BrainSchedulerTerminalEvent {
        run_id: s.run.run_id.clone(),
        operation_id: "a".into(),
        execution_kind: ExecutionKind::Agent,
        execution_id: "agent-a".into(),
        status: BrainOperationStatus::Error,
        source_sequence: 1,
    };
    let c = scheduler::terminal(&s, &n, 2).unwrap().unwrap();
    assert_eq!(c.run.phase, BrainSchedulerPhase::Failed);
    assert!(
        c.operations
            .iter()
            .find(|o| o.operation_id == "b")
            .unwrap()
            .cancel_requested
    );
}

#[test]
fn late_terminal_event_only_appends_audit_event() {
    let mut s = snapshot();
    s.run.phase = BrainSchedulerPhase::Failed;
    s.run.round = 1;
    s.operations = vec![BrainOperation {
        operation_id: "a".into(),
        run_id: s.run.run_id.clone(),
        round: 1,
        capability_id: "a".into(),
        execution_kind: ExecutionKind::Agent,
        execution_id: "agent-a".into(),
        status: BrainOperationStatus::Error,
        source_sequence: Some(2),
        cancel_requested: false,
    }];
    let notice = BrainSchedulerTerminalEvent {
        run_id: s.run.run_id.clone(),
        operation_id: "a".into(),
        execution_kind: ExecutionKind::Agent,
        execution_id: "agent-a".into(),
        status: BrainOperationStatus::Done,
        source_sequence: 3,
    };
    let change = scheduler::terminal(&s, &notice, 4).unwrap().unwrap();
    assert_eq!(change.operations, s.operations);
    assert_eq!(
        change.events[0].reason_summary.as_deref(),
        Some("late terminal event")
    );
}

#[test]
fn cancelled_sibling_late_event_settles_after_run_failure() {
    let mut s = snapshot();
    s.run.phase = BrainSchedulerPhase::Failed;
    s.run.round = 1;
    s.operations = vec![BrainOperation {
        operation_id: "sibling".into(),
        run_id: s.run.run_id.clone(),
        round: 1,
        capability_id: "sibling".into(),
        execution_kind: ExecutionKind::Agent,
        execution_id: "agent-sibling".into(),
        status: BrainOperationStatus::Running,
        source_sequence: None,
        cancel_requested: true,
    }];
    let notice = BrainSchedulerTerminalEvent {
        run_id: s.run.run_id.clone(),
        operation_id: "sibling".into(),
        execution_kind: ExecutionKind::Agent,
        execution_id: "agent-sibling".into(),
        status: BrainOperationStatus::Cancelled,
        source_sequence: 1,
    };
    let change = scheduler::terminal(&s, &notice, 4).unwrap().unwrap();
    assert_eq!(change.operations[0].status, BrainOperationStatus::Cancelled);
    assert_eq!(change.operations[0].source_sequence, Some(1));
    assert_eq!(change.events[0].reason_summary, None);
}

#[test]
fn objective_only_dispatch_is_allowed_only_without_required_inputs() {
    let mut request = request();
    request.inputs.clear();
    request.capability_ids = vec!["a".into()];
    let mut capability = cap("a");
    let decision = BrainSchedulerDecision::Dispatch {
        capabilities: vec![BrainDispatchItem {
            capability_id: "a".into(),
            inputs: BTreeMap::new(),
        }],
        reason: "perform the objective".into(),
        evidence_execution_ids: vec![],
    };
    assert!(scheduler::decide(&snapshot(), &request, &[capability.clone()], &decision, 2).is_err());
    capability.required_inputs.clear();
    assert!(scheduler::decide(&snapshot(), &request, &[capability.clone()], &decision, 2).is_ok());
    request.capability_ids = vec!["other".into()];
    assert!(scheduler::decide(&snapshot(), &request, &[capability], &decision, 2).is_err());
}

#[test]
fn saved_scheduler_plan_merges_inputs_and_requires_explicit_scope() {
    let mut plan: SchedulerPlan = serde_json::from_value(json!({"schema_version":3,"title":"Reusable","objective":"Verify","inputs":{"repo":"default","flag":true},"capability_ids":["a"]})).unwrap();
    scheduler::validate_plan(&plan).unwrap();
    let request = plan.request([("repo".into(), json!("override"))].into());
    assert_eq!(
        request.inputs,
        [
            ("repo".into(), json!("override")),
            ("flag".into(), json!(true))
        ]
        .into()
    );
    assert_eq!(request.max_rounds, 32);
    assert_eq!(plan.inputs["repo"], "default");
    plan.capability_ids.clear();
    assert!(scheduler::validate_plan(&plan).is_err());
}

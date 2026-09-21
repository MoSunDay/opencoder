use super::*;

pub(super) fn running() -> LayeredSnapshot {
    let snapshot = deciding();
    apply(
        &snapshot,
        layered::decide(
            &snapshot,
            &request(),
            &catalog(),
            &dispatch(1, &["impact", "audit"]),
            2,
        )
        .unwrap(),
    )
}

pub(super) fn operation<'a>(snapshot: &'a LayeredSnapshot, node: &str) -> &'a LayeredOperation {
    snapshot
        .operations
        .iter()
        .find(|op| op.node_id == node)
        .unwrap()
}

pub(super) fn force_deciding(snapshot: &LayeredSnapshot) -> LayeredSnapshot {
    let mut run = snapshot.run.clone();
    run.phase = LayeredPhase::Deciding;
    run.generation += 1;
    LayeredSnapshot {
        run,
        ..snapshot.clone()
    }
}

#[test]
fn parallel_nodes_in_layer_then_next_layer() {
    let mut snapshot = running();
    let impact = operation(&snapshot, "impact").clone();
    snapshot = apply(
        &snapshot,
        layered::terminal(
            &snapshot,
            &request(),
            &notice(&impact, LayeredOperationStatus::Done, 1),
            3,
        )
        .unwrap()
        .unwrap(),
    );
    assert_eq!(
        snapshot.run.phase,
        LayeredPhase::Waiting,
        "sibling still running"
    );

    let audit = operation(&snapshot, "audit").clone();
    snapshot = apply(
        &snapshot,
        layered::terminal(
            &snapshot,
            &request(),
            &notice(&audit, LayeredOperationStatus::Done, 1),
            4,
        )
        .unwrap()
        .unwrap(),
    );
    assert_eq!(snapshot.run.phase, LayeredPhase::Ready);
    assert!(snapshot
        .operations
        .iter()
        .all(|op| op.status == LayeredOperationStatus::Done));

    let snapshot = force_deciding(&snapshot);
    let context = layered::layer_context(
        &snapshot,
        &request(),
        &[cap("plan-fix", ExecutionKind::Agent, &[])],
        BTreeMap::new(),
        None,
    )
    .unwrap();
    assert_eq!(context.layer, 2);
    let fix = &context.nodes[0];
    assert_eq!(fix.node_id, "fix");
    assert_eq!(fix.upstream.len(), 2);
    assert!(fix
        .upstream
        .iter()
        .all(|upstream| upstream.execution_id.is_some()));
    assert_eq!(fix.downstream.len(), 1);
    assert_eq!(fix.downstream[0].node_id, "verify");

    let decision = LayeredDecision::DispatchLayer {
        layer: 2,
        assignments: vec![assignment(
            "fix",
            &operation(&snapshot, "impact").execution_id,
        )],
        reason: "bind impact".into(),
        evidence_execution_ids: vec![operation(&snapshot, "impact").execution_id.clone()],
    };
    let update = layered::decide(&snapshot, &request(), &catalog(), &decision, 5).unwrap();
    assert_eq!(update.run.layer, 2);
    assert_eq!(update.operations.len(), 3);
}

#[test]
fn binding_to_non_ancestor_rejected() {
    let request = branching_request();
    let snapshot = LayeredSnapshot {
        schema_version: 4,
        run: layered::initialize("brain-layered", &request, 1)
            .unwrap()
            .run,
        operations: vec![],
    };
    let mut snapshot = LayeredSnapshot {
        run: LayeredRun {
            phase: LayeredPhase::Deciding,
            ..snapshot.run.clone()
        },
        ..snapshot.clone()
    };
    snapshot = apply(
        &snapshot,
        layered::decide(
            &snapshot,
            &request,
            &catalog(),
            &dispatch(1, &["impact", "audit"]),
            2,
        )
        .unwrap(),
    );
    for node in ["impact", "audit"] {
        let op = operation(&snapshot, node).clone();
        snapshot = apply(
            &snapshot,
            layered::terminal(
                &snapshot,
                &request,
                &notice(&op, LayeredOperationStatus::Done, 1),
                3,
            )
            .unwrap()
            .unwrap(),
        );
    }
    let snapshot = force_deciding(&snapshot);
    // `audit` is a successful execution, but it is not an ancestor of `fix`.
    let sibling = operation(&snapshot, "audit").execution_id.clone();
    let decision = LayeredDecision::DispatchLayer {
        layer: 2,
        assignments: vec![
            assignment("fix", &sibling),
            LayeredAssignment {
                node_id: "check".into(),
                inputs: BTreeMap::new(),
                reason: "no upstream needed".into(),
            },
        ],
        reason: "bind sibling".into(),
        evidence_execution_ids: vec![sibling],
    };
    let error = layered::decide(&snapshot, &request, &catalog(), &decision, 4)
        .unwrap_err()
        .to_string();
    assert!(error.contains("not a successful ancestor"), "{error}");
}

#[test]
fn retry_schedules_next_attempt_then_fails_run() {
    let snapshot = running();
    let impact = operation(&snapshot, "impact").clone();
    let update = layered::terminal(
        &snapshot,
        &request(),
        &notice(&impact, LayeredOperationStatus::Error, 1),
        3,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        update.run.phase,
        LayeredPhase::Waiting,
        "retry keeps the layer waiting"
    );
    let retry = update
        .operations
        .iter()
        .find(|op| op.node_id == "impact" && op.attempt == 2)
        .expect("retry operation");
    assert_eq!(retry.status, LayeredOperationStatus::Creating);
    assert_ne!(retry.execution_id, impact.execution_id);
    assert!(update
        .events
        .iter()
        .any(|event| event.event_type == "operation_retry_scheduled"));

    let snapshot = apply(&snapshot, update);
    let retry = snapshot
        .operations
        .iter()
        .find(|op| op.node_id == "impact" && op.attempt == 2)
        .unwrap()
        .clone();
    let update = layered::terminal(
        &snapshot,
        &request(),
        &notice(&retry, LayeredOperationStatus::Error, 1),
        4,
    )
    .unwrap()
    .unwrap();
    assert_eq!(update.run.phase, LayeredPhase::Failed);
    assert!(update.run.error.as_deref().unwrap().contains("attempt"));
    assert!(
        update
            .operations
            .iter()
            .find(|op| op.node_id == "audit")
            .unwrap()
            .cancel_requested
    );
    assert!(update
        .events
        .iter()
        .any(|event| event.event_type == "run_failed"));
}

#[test]
fn late_terminal_from_previous_attempt_ignored() {
    let snapshot = running();
    let impact = operation(&snapshot, "impact").clone();
    let retried = apply(
        &snapshot,
        layered::terminal(
            &snapshot,
            &request(),
            &notice(&impact, LayeredOperationStatus::Error, 1),
            3,
        )
        .unwrap()
        .unwrap(),
    );
    // A replayed delivery of the superseded attempt is a no-op.
    assert!(layered::terminal(
        &retried,
        &request(),
        &notice(&impact, LayeredOperationStatus::Done, 1),
        4
    )
    .unwrap()
    .is_none());

    // A notice carrying a foreign execution id is rejected instead of folding
    // into the live attempt.
    let retry = retried
        .operations
        .iter()
        .find(|op| op.node_id == "impact" && op.attempt == 2)
        .unwrap()
        .clone();
    let mut spoofed = notice(&retry, LayeredOperationStatus::Done, 1);
    spoofed.execution_id = impact.execution_id.clone();
    assert!(layered::terminal(&retried, &request(), &spoofed, 4)
        .unwrap_err()
        .to_string()
        .contains("identity mismatch"));

    // Sequences are per execution: the retry folds its own first notice.
    let completed = apply(
        &retried,
        layered::terminal(
            &retried,
            &request(),
            &notice(&retry, LayeredOperationStatus::Done, 1),
            5,
        )
        .unwrap()
        .unwrap(),
    );
    assert!(layered::terminal(
        &completed,
        &request(),
        &notice(&impact, LayeredOperationStatus::Error, 1),
        6
    )
    .unwrap()
    .is_none());
    assert_eq!(
        completed
            .operations
            .iter()
            .filter(|op| op.node_id == "impact")
            .count(),
        2
    );
}

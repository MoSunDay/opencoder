use super::*;

#[test]
fn forward_after_rework_rejects_copied_reflection_and_accepts_null_without_another_round() {
    let req = request();
    let initial = snap(initialize("brain-rework-forward", &req, 1).unwrap());
    let first = finish(
        snap(dispatch(&initial, &req, 1)),
        &req,
        LayeredOperationStatus::Done,
    );
    let failed = finish(
        snap(dispatch(&first, &req, 2)),
        &req,
        LayeredOperationStatus::Error,
    );
    let mut repaired = finish(
        snap(dispatch(&failed, &req, 1)),
        &req,
        LayeredOperationStatus::Done,
    );
    repaired.run.phase = LayeredPhase::Deciding;
    let mut copied = proposal(&repaired, &req, 2);
    let LayeredDecision::DispatchLayer { reflection, .. } = &mut copied else {
        unreachable!()
    };
    *reflection = repaired.run.reflection.clone();
    assert!(reflection.is_some());
    let error = decide(&repaired, &req, &catalog(), &copied, 30)
        .unwrap_err()
        .to_string();
    assert!(error.contains("set reflection to null"), "{error}");
    let corrected = decide(
        &repaired,
        &req,
        &catalog(),
        &proposal(&repaired, &req, 2),
        31,
    )
    .unwrap();
    assert_eq!(corrected.run.layer, 2);
    assert_eq!(corrected.run.round, 2);
    assert_eq!(corrected.run.activation, 4);
    assert_eq!(corrected.run.valid_layers, 1);
    assert_eq!(corrected.run.reflection, repaired.run.reflection);
    assert_eq!(corrected.operations.len(), repaired.operations.len() + 1);
}

#[test]
fn stopped_shared_run_recovers_same_execution_without_spending_a_round() {
    let mut req = request();
    req.plan.shared_workspace = true;
    let mut caps = catalog();
    for cap in &mut caps {
        cap.kind = opencoder_core::fleet::ExecutionKind::Operator;
    }
    let mut initial = snap(initialize("brain-recovery", &req, 1).unwrap());
    initial.run.phase = LayeredPhase::Deciding;
    let admitted = snap(decide(&initial, &req, &caps, &proposal(&initial, &req, 1), 2).unwrap());
    let failed = finish(admitted, &req, LayeredOperationStatus::Error);
    for phase in [LayeredPhase::Blocked, LayeredPhase::Paused] {
        let mut stopped = failed.clone();
        stopped.run.phase = phase;
        let identities: Vec<_> = stopped
            .operations
            .iter()
            .map(|o| o.execution_id.clone())
            .collect();
        for operation in &failed.operations {
            let mut receipt = notice(operation, LayeredOperationStatus::Done);
            assert!(terminal(&stopped, &req, &receipt, 3).unwrap().is_none());
            receipt.source_sequence = 2;
            let mut ordinary = req.clone();
            ordinary.plan.shared_workspace = false;
            assert!(terminal(&stopped, &ordinary, &receipt, 3)
                .unwrap()
                .is_none());
            let mut obsolete = stopped.clone();
            obsolete.run.activation += 1;
            assert!(terminal(&obsolete, &req, &receipt, 3).unwrap().is_none());
            let mut active = stopped.clone();
            active.run.phase = LayeredPhase::Ready;
            assert!(terminal(&active, &req, &receipt, 3).unwrap().is_none());
            let change = terminal(&stopped, &req, &receipt, 3).unwrap().unwrap();
            assert_eq!(change.events[0].event_type, "operation_terminal");
            stopped = snap(change);
            assert_eq!(stopped.run.phase, phase);
            assert_eq!(stopped.run.round, 1);
            assert_eq!(stopped.run.activation, 1);
            assert!(terminal(&stopped, &req, &receipt, 4).unwrap().is_none());
            receipt.source_sequence = 3;
            assert!(terminal(&stopped, &req, &receipt, 4).unwrap().is_none());
        }
        assert_eq!(
            stopped
                .operations
                .iter()
                .map(|o| o.execution_id.clone())
                .collect::<Vec<_>>(),
            identities
        );
        let resumed = snap(command(&stopped, &req.plan, "resume", 5).unwrap());
        assert_eq!(resumed.run.phase, LayeredPhase::Ready);
        assert!(resumed.operations.iter().all(|o| o.status.successful()));
        assert_eq!(resumed.run.round, 1);
    }
}

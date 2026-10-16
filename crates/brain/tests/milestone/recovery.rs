use super::*;

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

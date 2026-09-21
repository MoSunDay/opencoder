use super::*;

fn dispatch(ids: &[&str]) -> BrainSchedulerDecision {
    BrainSchedulerDecision::Dispatch {
        capabilities: ids
            .iter()
            .map(|id| BrainDispatchItem {
                capability_id: (*id).into(),
                inputs: [(
                    "repo".into(),
                    BrainInputBinding::Root {
                        name: "repo".into(),
                    },
                )]
                .into(),
            })
            .collect(),
        reason: "parallel tests".into(),
        evidence_execution_ids: vec![],
    }
}

fn parallel() -> BrainSchedulerSnapshot {
    let change = scheduler::decide(
        &snapshot(),
        &request(),
        &[cap("a"), cap("b"), cap("c")],
        &dispatch(&["a", "b", "c"]),
        2,
    )
    .unwrap();
    BrainSchedulerSnapshot {
        schema_version: 3,
        run: change.run,
        operations: change.operations,
    }
}

fn notice(state: &BrainSchedulerSnapshot, index: usize, seq: u64) -> BrainSchedulerTerminalEvent {
    let op = &state.operations[index];
    BrainSchedulerTerminalEvent {
        run_id: state.run.run_id.clone(),
        operation_id: op.operation_id.clone(),
        execution_kind: op.execution_kind,
        execution_id: op.execution_id.clone(),
        status: BrainOperationStatus::Done,
        source_sequence: seq,
    }
}

fn fold(state: &mut BrainSchedulerSnapshot, change: BrainSchedulerChange) {
    state.run = change.run;
    state.operations = change.operations;
}

#[test]
fn only_last_success_wakes_round_and_replays_do_not_duplicate_barrier() {
    let mut state = parallel();
    for (position, index) in [2, 0, 1].into_iter().enumerate() {
        let event = notice(&state, index, 10);
        let change = scheduler::terminal(&state, &event, 3).unwrap().unwrap();
        assert_eq!(
            change
                .events
                .iter()
                .filter(|e| e.event_type == "round_barrier_reached")
                .count(),
            usize::from(position == 2)
        );
        assert_eq!(
            change.run.phase,
            if position == 2 {
                BrainSchedulerPhase::Ready
            } else {
                BrainSchedulerPhase::Waiting
            }
        );
        fold(&mut state, change);
        assert!(scheduler::terminal(&state, &event, 4).unwrap().is_none());
        assert!(scheduler::terminal(&state, &notice(&state, index, 9), 4)
            .unwrap()
            .is_none());
    }
    let late = scheduler::terminal(&state, &notice(&state, 0, 11), 5)
        .unwrap()
        .unwrap();
    assert_eq!(late.events.len(), 1);
    assert_eq!(
        late.events[0].reason_summary.as_deref(),
        Some("late terminal event")
    );
    assert_eq!(late.operations, state.operations);
}

#[test]
fn pause_preserves_terminal_barrier_and_resume_wakes_once() {
    let mut state = parallel();
    let paused = scheduler::command(&state, "pause", 3).unwrap();
    fold(&mut state, paused);
    for index in 0..3 {
        let change = scheduler::terminal(&state, &notice(&state, index, 1), 4)
            .unwrap()
            .unwrap();
        assert_eq!(change.run.phase, BrainSchedulerPhase::Paused);
        fold(&mut state, change);
    }
    let resumed = scheduler::command(&state, "resume", 5).unwrap();
    assert_eq!(resumed.run.phase, BrainSchedulerPhase::Ready);
    fold(&mut state, resumed);
    assert!(scheduler::command(&state, "resume", 6).is_err());
}

#[test]
fn resume_clears_blocked_error_without_replacing_execution_history() {
    let mut state = parallel();
    for index in 0..3 {
        let change = scheduler::terminal(&state, &notice(&state, index, 1), 3)
            .unwrap()
            .unwrap();
        fold(&mut state, change);
    }
    let operations = state.operations.clone();
    let run_id = state.run.run_id.clone();
    let round = state.run.round;
    let reason = "model provider returned HTTP 429";
    let blocked = scheduler::block(&state, reason.into(), 4);
    let diagnostic = blocked.events[0].clone();
    fold(&mut state, blocked);
    assert!(scheduler::command(&state, "resume", 5).is_err());
    let paused = scheduler::command(&state, "pause", 5).unwrap();
    assert_eq!(paused.run.error.as_deref(), Some(reason));
    fold(&mut state, paused);

    let resumed = scheduler::command(&state, "resume", 6).unwrap();
    assert_eq!(resumed.run.phase, BrainSchedulerPhase::Ready);
    assert_eq!(resumed.run.error, None);
    assert_eq!(resumed.run.run_id, run_id);
    assert_eq!(resumed.run.round, round);
    assert_eq!(resumed.operations, operations);
    assert_eq!(resumed.events.len(), 1);
    assert_eq!(resumed.events[0].event_type, "run_resumed");
    assert_eq!(diagnostic.event_type, "decision_blocked");
    assert_eq!(diagnostic.reason_summary.as_deref(), Some(reason));
}

#[test]
fn old_round_and_mismatched_identity_cannot_advance_current_round() {
    let mut state = parallel();
    state.run.round = 2;
    let event = notice(&state, 0, 1);
    let change = scheduler::terminal(&state, &event, 3).unwrap().unwrap();
    assert_eq!(change.run.phase, BrainSchedulerPhase::Waiting);
    assert_eq!(change.operations, state.operations);
    assert_eq!(change.events[0].round, 1);
    let mut wrong = event.clone();
    wrong.execution_id = "agent-other".into();
    assert!(scheduler::terminal(&state, &wrong, 3).is_err());
    wrong = event;
    wrong.status = BrainOperationStatus::Running;
    assert!(scheduler::terminal(&state, &wrong, 3).is_err());
}

#[test]
fn duplicate_dispatch_active_work_and_round_limit_reject_decisions() {
    let catalog = [cap("a")];
    assert!(
        scheduler::decide(&snapshot(), &request(), &catalog, &dispatch(&["a", "a"]), 2).is_err()
    );
    let mut state = parallel();
    state.run.phase = BrainSchedulerPhase::Deciding;
    let complete = BrainSchedulerDecision::Complete {
        reason: "verified".into(),
        evidence_execution_ids: vec![state.operations[0].execution_id.clone()],
    };
    assert!(scheduler::decide(&state, &request(), &catalog, &complete, 3).is_err());
    for operation in &mut state.operations {
        operation.status = BrainOperationStatus::Done;
    }
    state.run.round = request().max_rounds;
    assert!(scheduler::decide(&state, &request(), &catalog, &dispatch(&["a"]), 3).is_err());
    assert_eq!(
        scheduler::decide(&state, &request(), &catalog, &complete, 3)
            .unwrap()
            .run
            .phase,
        BrainSchedulerPhase::Completed
    );
    state.run.round += 1;
    assert!(scheduler::decide(&state, &request(), &catalog, &complete, 3).is_err());
    assert!(scheduler::decide(
        &snapshot(),
        &request(),
        &catalog,
        &BrainSchedulerDecision::Complete {
            reason: "verified".into(),
            evidence_execution_ids: vec![]
        },
        3
    )
    .is_err());
}

#[test]
fn reference_validation_and_execution_identity_are_deterministic() {
    let catalog = [cap("a")];
    let first = scheduler::decide(&snapshot(), &request(), &catalog, &dispatch(&["a"]), 2).unwrap();
    let replay =
        scheduler::decide(&snapshot(), &request(), &catalog, &dispatch(&["a"]), 99).unwrap();
    assert_eq!(first.operations, replay.operations);
    let mut state = parallel();
    state.run.phase = BrainSchedulerPhase::Deciding;
    for operation in &mut state.operations {
        operation.status = BrainOperationStatus::Done;
    }
    let execution = state.operations[0].execution_id.clone();
    for (binding, valid) in [
        (
            BrainInputBinding::Root {
                name: "missing".into(),
            },
            false,
        ),
        (
            BrainInputBinding::Artifact {
                reference: "missing".into(),
            },
            false,
        ),
        (
            BrainInputBinding::Execution {
                execution_id: execution.clone(),
                path: "/a~1b/~0value".into(),
            },
            true,
        ),
        (
            BrainInputBinding::Execution {
                execution_id: execution.clone(),
                path: "/bad~2".into(),
            },
            false,
        ),
        (
            BrainInputBinding::Execution {
                execution_id: "agent-missing".into(),
                path: "".into(),
            },
            false,
        ),
    ] {
        let mut decision = dispatch(&["a"]);
        if let BrainSchedulerDecision::Dispatch { capabilities, .. } = &mut decision {
            capabilities[0].inputs.insert("repo".into(), binding);
        }
        assert_eq!(
            scheduler::decide(&state, &request(), &catalog, &decision, 3).is_ok(),
            valid
        );
    }
}

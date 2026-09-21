use super::barriers::{force_deciding, operation, running};
use super::*;

#[test]
fn completion_requires_every_layer_and_freezes_the_summary() {
    let mut snapshot = running();
    for node in ["impact", "audit"] {
        let op = operation(&snapshot, node).clone();
        snapshot = apply(
            &snapshot,
            layered::terminal(
                &snapshot,
                &request(),
                &notice(&op, LayeredOperationStatus::Done, 1),
                3,
            )
            .unwrap()
            .unwrap(),
        );
    }
    let snapshot = force_deciding(&snapshot);
    let error = layered::decide(
        &snapshot,
        &request(),
        &catalog(),
        &LayeredDecision::Complete {
            reason: "done".into(),
            evidence_execution_ids: vec![],
            summary: "early".into(),
        },
        4,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("every layer"), "{error}");

    let mut snapshot = snapshot;
    snapshot = apply(
        &snapshot,
        layered::decide(&snapshot, &request(), &catalog(), &dispatch(2, &["fix"]), 4).unwrap(),
    );
    let fix = operation(&snapshot, "fix").clone();
    snapshot = apply(
        &snapshot,
        layered::terminal(
            &snapshot,
            &request(),
            &notice(&fix, LayeredOperationStatus::Done, 1),
            5,
        )
        .unwrap()
        .unwrap(),
    );
    let mut snapshot = force_deciding(&snapshot);
    snapshot = apply(
        &snapshot,
        layered::decide(
            &snapshot,
            &request(),
            &catalog(),
            &dispatch(3, &["verify"]),
            6,
        )
        .unwrap(),
    );
    let verify = operation(&snapshot, "verify").clone();
    snapshot = apply(
        &snapshot,
        layered::terminal(
            &snapshot,
            &request(),
            &notice(&verify, LayeredOperationStatus::Done, 1),
            7,
        )
        .unwrap()
        .unwrap(),
    );
    let snapshot = force_deciding(&snapshot);
    let update = layered::decide(
        &snapshot,
        &request(),
        &catalog(),
        &LayeredDecision::Complete {
            reason: "verified".into(),
            evidence_execution_ids: vec![verify.execution_id.clone()],
            summary: "repaired and verified".into(),
        },
        8,
    )
    .unwrap();
    assert_eq!(update.run.phase, LayeredPhase::Completed);
    assert_eq!(update.run.summary.as_deref(), Some("repaired and verified"));
}

#[test]
fn cancel_and_pause_commands_stop_dispatch() {
    let snapshot = running();
    let cancelled = layered::command(&snapshot, &request().plan, "cancel", 3).unwrap();
    assert_eq!(cancelled.run.phase, LayeredPhase::Cancelled);
    assert!(cancelled.operations.iter().all(|op| op.cancel_requested));

    let paused = layered::command(&snapshot, &request().plan, "pause", 4).unwrap();
    assert_eq!(paused.run.phase, LayeredPhase::Paused);
    let resumed =
        layered::command(&apply(&snapshot, paused), &request().plan, "resume", 5).unwrap();
    assert_eq!(
        resumed.run.phase,
        LayeredPhase::Waiting,
        "layer is unfinished"
    );
    assert!(layered::command(&apply(&snapshot, cancelled), &request().plan, "pause", 6).is_err());
}

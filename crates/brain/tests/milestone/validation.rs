use super::*;
#[test]
fn rejects_incomplete_or_forged_dispatches_and_bad_input_bindings() {
    let req = request();
    let mut initial = snap(initialize("brain-validation", &req, 1).unwrap());
    initial.run.phase = LayeredPhase::Deciding;
    let baseline = serde_json::to_value(proposal(&initial, &req, 1)).unwrap();
    for (pointer, value, expected) in [
        (
            "/assignments/0/capability_id",
            json!("foreign"),
            "not attached",
        ),
        ("/assignments/0/node_id", json!("test"), "outside target"),
        (
            "/assignments/0/inputs",
            json!({"task":{"kind":"root","name":"missing"}}),
            "unknown root",
        ),
        (
            "/assignments/0/inputs",
            json!({"task":{"kind":"artifact","reference":"missing"}}),
            "unknown artifact",
        ),
        (
            "/assignments/0/inputs",
            json!({"task":{"kind":"execution","execution_id":"agent-fake","path":""}}),
            "terminal executions",
        ),
        ("/layer", json!(2), "skip layers"),
    ] {
        let mut value_json = baseline.clone();
        *value_json.pointer_mut(pointer).unwrap() = value;
        let decision = serde_json::from_value(value_json).unwrap();
        assert!(
            decide(&initial, &req, &catalog(), &decision, 2)
                .unwrap_err()
                .to_string()
                .contains(expected),
            "{pointer}"
        );
    }
    let mut missing = baseline.clone();
    missing["assignments"].as_array_mut().unwrap().pop();
    assert!(decide(
        &initial,
        &req,
        &catalog(),
        &serde_json::from_value(missing).unwrap(),
        2
    )
    .unwrap_err()
    .to_string()
    .contains("every milestone"));
    let mut duplicate = baseline.clone();
    duplicate["assignments"][1] = duplicate["assignments"][0].clone();
    assert!(decide(
        &initial,
        &req,
        &catalog(),
        &serde_json::from_value(duplicate).unwrap(),
        2
    )
    .unwrap_err()
    .to_string()
    .contains("duplicate"));
    let mut caps = catalog();
    caps[0].required_inputs.push("task".into());
    assert!(
        decide(&initial, &req, &caps, &proposal(&initial, &req, 1), 2)
            .unwrap_err()
            .to_string()
            .contains("missing required")
    );
}
#[test]
fn partial_barrier_and_failed_milestone_cannot_advance_but_can_return() {
    let req = request();
    let initial = snap(initialize("brain-gates", &req, 1).unwrap());
    let mut waiting = snap(dispatch(&initial, &req, 1));
    waiting.run.phase = LayeredPhase::Deciding;
    assert!(
        decide(&waiting, &req, &catalog(), &proposal(&waiting, &req, 2), 2)
            .unwrap_err()
            .to_string()
            .contains("terminate")
    );
    let mut failed = finish(waiting, &req, LayeredOperationStatus::Error);
    failed.run.phase = LayeredPhase::Deciding;
    assert!(
        decide(&failed, &req, &catalog(), &proposal(&failed, &req, 2), 3)
            .unwrap_err()
            .to_string()
            .contains("did not meet")
    );
    let returned = decide(&failed, &req, &catalog(), &proposal(&failed, &req, 1), 4).unwrap();
    assert_eq!(returned.run.round, 2);
    assert_eq!(returned.run.layer, 1);
}
#[test]
fn pause_retains_the_barrier_and_cancellation_accepts_a_racing_success_receipt() {
    let req = request();
    let initial = snap(initialize("brain-pause", &req, 1).unwrap());
    let running = snap(dispatch(&initial, &req, 1));
    let paused = snap(command(&running, &req.plan, "pause", 11).unwrap());
    let ended = finish(paused, &req, LayeredOperationStatus::Done);
    assert_eq!(ended.run.phase, LayeredPhase::Paused);
    assert_eq!(
        command(&ended, &req.plan, "resume", 30).unwrap().run.phase,
        LayeredPhase::Ready
    );
    let cancelled = snap(command(&running, &req.plan, "cancel", 11).unwrap());
    let ended = finish(cancelled, &req, LayeredOperationStatus::Done);
    assert_eq!(ended.run.phase, LayeredPhase::Cancelled);
    assert!(ended.operations.iter().all(|op| op.status.terminal()));
}
#[test]
fn context_keeps_latest_visits_and_shared_capabilities_once() {
    let req = request();
    let initial = snap(initialize("brain-context", &req, 1).unwrap());
    let first = finish(
        snap(dispatch(&initial, &req, 1)),
        &req,
        LayeredOperationStatus::Error,
    );
    let next = snap(dispatch(&first, &req, 1));
    let ctx = layer_context(&next, &req, &catalog(), Default::default(), None).unwrap();
    assert_eq!(ctx.capabilities.len(), 2);
    assert_eq!(ctx.operations.len(), 3);
    assert!(ctx.operations.iter().all(|op| op.activation == 2));
    assert_eq!(next.operations.len(), 6, "history remains intact");
    let mut huge = ctx;
    huge.request
        .inputs
        .insert("oversized".into(), json!("x".repeat(1024 * 1024)));
    assert!(instruction(&huge)
        .unwrap_err()
        .to_string()
        .contains("1 MiB"));
}

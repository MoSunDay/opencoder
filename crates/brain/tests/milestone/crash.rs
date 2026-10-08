use super::*;

#[test]
fn initial_or_resumed_context_explains_one_based_dispatch_numbers() {
    let req = request();
    let root = snap(initialize("brain-numbering", &req, 1).unwrap());
    let blocked = snap(block(&root, "missing prerequisite".into(), 2));
    let mut resumed = snap(command(&blocked, &req.plan, "resume", 3).unwrap());
    resumed.run.phase = LayeredPhase::Deciding;
    let context = layer_context(&resumed, &req, &catalog(), Default::default(), None).unwrap();
    let text = instruction(&context).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["run"]["layer"], 0);
    assert_eq!(
        value["layer_catalog"],
        json!([
            {"number":1,"layer_id":"coding"}, {"number":2,"layer_id":"testing"}
        ])
    );
    let mut decision = proposal(&resumed, &req, 1);
    if let LayeredDecision::DispatchLayer { layer, .. } = &mut decision {
        *layer = 0;
    }
    let error = decide(&resumed, &req, &catalog(), &decision, 4)
        .unwrap_err()
        .to_string();
    assert!(error.contains("valid dispatch layers are 1..=2"), "{error}");
    assert_eq!(
        decide(&resumed, &req, &catalog(), &proposal(&resumed, &req, 1), 5)
            .unwrap()
            .run
            .layer,
        1
    );
}

#[test]
fn crash_fails_root_immediately_preserves_children_and_late_results_never_revive_it() {
    let req = request();
    let root = snap(initialize("brain-crash", &req, 1).unwrap());
    let waiting = snap(dispatch(&root, &req, 1));
    assert!(!barrier(&waiting));
    let failed = snap(fail(&waiting, "Worker crashed".into(), 2).unwrap());
    assert_eq!(failed.run.phase, LayeredPhase::Failed);
    assert_eq!(
        serde_json::to_value(&failed.operations).unwrap(),
        serde_json::to_value(&waiting.operations).unwrap()
    );
    assert!(fail(&failed, "again".into(), 3).is_none());
    assert!(command(&failed, &req.plan, "resume", 4).is_err());
    let settled = finish(failed, &req, LayeredOperationStatus::Done);
    assert_eq!(settled.run.phase, LayeredPhase::Failed);
    assert_eq!(settled.run.activation, 1);
    assert!(settled
        .operations
        .iter()
        .all(|op| op.status == LayeredOperationStatus::Done && !op.cancel_requested));
    assert!(terminal(
        &settled,
        &req,
        &notice(&settled.operations[0], LayeredOperationStatus::Done),
        30
    )
    .unwrap()
    .is_none());
}

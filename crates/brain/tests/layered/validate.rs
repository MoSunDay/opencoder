use super::*;

#[test]
fn kahn_levels_follow_edges_not_json_order() {
    let levels = layered::layers(&plan()).unwrap();
    assert_eq!(
        levels,
        vec![
            vec!["impact".to_string(), "audit".to_string()],
            vec!["fix".to_string()],
            vec!["verify".to_string()],
        ]
    );
    assert_eq!(layered::layer_of(&plan(), "fix").unwrap(), 2);
    let ancestors = layered::ancestors(&plan()).unwrap();
    assert_eq!(
        ancestors["verify"].iter().cloned().collect::<Vec<_>>(),
        vec!["audit".to_string(), "fix".to_string(), "impact".to_string()]
    );
}

#[test]
fn node_requires_exactly_one_capability_and_unknown_edges_fail() {
    let mut broken = plan();
    broken.nodes[0].capability_id = "  ".into();
    assert!(layered::validate_plan(&broken)
        .unwrap_err()
        .to_string()
        .contains("exactly one capability"));

    let mut broken = plan();
    broken.edges.push(edge("impact", "ghost"));
    assert!(layered::layers(&broken)
        .unwrap_err()
        .to_string()
        .contains("unknown node"));

    let mut cyclic = plan();
    cyclic.edges.push(edge("verify", "impact"));
    assert!(layered::validate_plan(&cyclic)
        .unwrap_err()
        .to_string()
        .contains("cycle"));

    let mut wide = plan();
    for index in 0..33 {
        wide.nodes.push(node(&format!("n{index}"), "agent-impact"));
    }
    assert!(layered::validate_plan(&wide)
        .unwrap_err()
        .to_string()
        .contains("layer width"));
}

#[test]
fn plan_rejects_unknown_fields_and_foreign_todo() {
    let value = serde_json::to_value(plan()).unwrap();
    let mut extra = value.clone();
    extra["stray"] = json!(true);
    assert!(serde_json::from_value::<LayeredPlan>(extra).is_err());

    let mut foreign = plan();
    foreign.todo = Some(LayeredTodoRef {
        id: "todo-1".into(),
    });
    assert!(layered::validate_plan(&foreign)
        .unwrap_err()
        .to_string()
        .contains("project todo id"));
}

#[test]
fn request_rejects_unknown_schema_and_lost_parent() {
    let mut wrong = request();
    wrong.schema_version = 3;
    assert!(layered::validate_request(&wrong).is_err());

    let mut nested = request();
    nested.depth = 1;
    assert!(layered::validate_request(&nested)
        .unwrap_err()
        .to_string()
        .contains("parent"));
    nested.parent = Some(LayeredParent {
        run_id: "brain-parent".into(),
        operation_id: "brain-parent#l1#fix#a1".into(),
        node_id: "fix".into(),
        layer: 1,
    });
    assert!(layered::validate_request(&nested).is_ok());

    nested.depth = 4;
    assert!(layered::validate_request(&nested)
        .unwrap_err()
        .to_string()
        .contains("depth"));
}

#[test]
fn dispatch_covers_the_layer_and_fixes_bindings() {
    let snapshot = deciding();
    let error = layered::decide(
        &snapshot,
        &request(),
        &catalog(),
        &dispatch(1, &["impact"]),
        2,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("cover exactly"), "{error}");

    let error = layered::decide(
        &snapshot,
        &request(),
        &catalog(),
        &dispatch(2, &["impact", "audit"]),
        2,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("next layer"), "{error}");

    let blank = LayeredDecision::DispatchLayer {
        layer: 1,
        assignments: vec![LayeredAssignment {
            node_id: "impact".into(),
            inputs: BTreeMap::new(),
            reason: "  ".into(),
        }],
        reason: "dispatch".into(),
        evidence_execution_ids: vec![],
    };
    assert!(layered::decide(&snapshot, &request(), &catalog(), &blank, 2).is_err());

    let update = layered::decide(
        &snapshot,
        &request(),
        &catalog(),
        &dispatch(1, &["impact", "audit"]),
        2,
    )
    .unwrap();
    assert_eq!(update.run.layer, 1);
    assert_eq!(update.run.phase, LayeredPhase::Waiting);
    assert_eq!(update.operations.len(), 2);
    assert!(update
        .operations
        .iter()
        .all(|op| op.attempt == 1 && op.status == LayeredOperationStatus::Creating));
    assert!(update
        .events
        .iter()
        .any(|event| event.event_type == "layer_started"));
    assert_eq!(
        update
            .events
            .iter()
            .filter(|event| event.event_type == "node_dispatched")
            .count(),
        2
    );
}

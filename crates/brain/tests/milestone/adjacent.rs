use super::*;

#[test]
fn adjacent_workflow_rejects_skipped_return_and_invalidates_later_passes() {
    let mut req = request();
    req.plan.adjacent_only = true;
    let mut last = req.plan.layers[1].clone();
    last.layer_id = "review".into();
    req.plan.layers.push(last);
    let mut node = req.plan.nodes[2].clone();
    node.node_id = "review".into();
    node.layer_id = "review".into();
    req.plan.nodes.push(node);
    let mut current = snap(initialize("brain-adjacent", &req, 1).unwrap());
    for layer in 1..=3 {
        current = finish(
            snap(dispatch(&current, &req, layer)),
            &req,
            LayeredOperationStatus::Done,
        );
    }
    current.run.phase = LayeredPhase::Deciding;
    let skipped = decide(&current, &req, &catalog(), &proposal(&current, &req, 1), 30).unwrap_err();
    assert!(skipped.to_string().contains("adjacent"));
    let returned = snap(dispatch(&current, &req, 2));
    assert_eq!(returned.run.round, 2);
    assert_eq!(returned.run.valid_layers, 1);
    assert_eq!(returned.run.activation, 4);
    assert_eq!(returned.operations.len(), 5);
    assert_ne!(
        returned.operations.last().unwrap().execution_id,
        current.operations[2].execution_id
    );
    let mut done = finish(returned, &req, LayeredOperationStatus::Done);
    // A new verification must not bind the previous repair's superseded output.
    let mut shared = req.clone();
    shared.plan.shared_workspace = true;
    let mut deciding = done.clone();
    deciding.run.phase = LayeredPhase::Deciding;
    let mut proposed = proposal(&deciding, &shared, 3);
    if let LayeredDecision::DispatchLayer { assignments, .. } = &mut proposed {
        assignments[0].inputs.insert(
            "repair".into(),
            serde_json::from_value(json!({
                "kind":"execution", "execution_id":current.operations[2].execution_id,"path":""
            }))
            .unwrap(),
        );
    }
    let mut operators = catalog();
    for cap in &mut operators {
        cap.kind = opencoder_core::fleet::ExecutionKind::Operator;
    }
    assert!(decide(&deciding, &shared, &operators, &proposed, 40)
        .unwrap_err()
        .to_string()
        .contains("obsolete"));
    done = snap(dispatch(&done, &req, 1));
    assert_eq!(done.run.valid_layers, 0);
    assert_eq!(done.run.round, 3);
}

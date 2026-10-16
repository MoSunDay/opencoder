use super::*;

#[test]
fn legacy_context_keeps_the_old_wire_and_cannot_publish_artifact_bindings() {
    let request = request();
    let snapshot = snap(initialize("brain-wire", &request, 1).unwrap());
    let mut context =
        layer_context(&snapshot, &request, &catalog(), Default::default(), None).unwrap();
    let legacy = serde_json::to_value(&context).unwrap();
    assert!(legacy.get("execution_io_version").is_none());
    assert!(legacy.get("execution_artifacts").is_none());
    assert!(
        serde_json::from_str::<serde_json::Value>(&instruction(&context).unwrap())
            .unwrap()
            .get("artifact_bindings")
            .is_none()
    );
    let mut decision = proposal(&snapshot, &request, 1);
    if let LayeredDecision::DispatchLayer { assignments, .. } = &mut decision {
        assignments[0].inputs.insert("files".into(), serde_json::from_value(json!({"kind":"execution","execution_id":"agent-done","source":"artifacts","path":""})).unwrap());
    }
    assert!(validate_execution_io(0, &decision)
        .unwrap_err()
        .to_string()
        .contains("frozen at admission"));
    validate_execution_io(1, &decision).unwrap();
    context.execution_io_version = 1;
    let modern: serde_json::Value = serde_json::from_str(&instruction(&context).unwrap()).unwrap();
    assert_eq!(
        modern["artifact_bindings"]["example"]["source"],
        "artifacts"
    );
    assert_eq!(
        serde_json::to_value(&context).unwrap()["execution_io_version"],
        1
    );
}

#[test]
fn ordinary_and_shared_plans_reject_foreign_and_old_inputs_but_allow_latest_failure() {
    let req = request();
    let first = finish(
        snap(dispatch(
            &snap(initialize("brain-bindings", &req, 1).unwrap()),
            &req,
            1,
        )),
        &req,
        LayeredOperationStatus::Done,
    );
    let old = first.operations[0].execution_id.clone();
    let mut current = finish(
        snap(dispatch(&first, &req, 1)),
        &req,
        LayeredOperationStatus::Error,
    );
    current.run.phase = LayeredPhase::Deciding;
    let latest = current
        .operations
        .iter()
        .rfind(|op| op.node_id == "code")
        .unwrap()
        .execution_id
        .clone();
    for shared in [false, true] {
        let mut request = req.clone();
        request.plan.shared_workspace = shared;
        let mut capabilities = catalog();
        if shared {
            for cap in &mut capabilities {
                cap.kind = opencoder_core::fleet::ExecutionKind::Operator;
            }
        }
        for (id, expected) in [
            (&old, Some("obsolete")),
            (&"operator-another-run".into(), Some("terminal executions")),
            (&latest, None),
        ] {
            let mut proposed = proposal(&current, &request, 1);
            if let LayeredDecision::DispatchLayer { assignments, .. } = &mut proposed {
                assignments[0].inputs.insert(
                    "diagnostic".into(),
                    serde_json::from_value(json!({
                        "kind":"execution","execution_id":id,"path":"","source":"artifacts"
                    }))
                    .unwrap(),
                );
            }
            let result = decide(&current, &request, &capabilities, &proposed, 40);
            match expected {
                Some(message) => assert!(result.unwrap_err().to_string().contains(message)),
                None => assert_eq!(result.unwrap().run.round, current.run.round + 1),
            }
        }
    }
}

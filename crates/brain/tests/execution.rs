mod support;
use opencoder_brain::execution::*;
use opencoder_core::brain::*;
use serde_json::json;
use support::*;

#[test]
fn successor_starts_while_an_independent_sibling_is_running() {
    let a = step("a");
    let b = step("b");
    let mut c = step("c");
    c["inputs"] =
        json!({"text":{"schema":{"type":"string"},"binding":{"source":"output","step":"a"}}});
    let mut run = run(plan(
        json!([a, b, c]),
        json!({"source":"output","step":"c"}),
    ));
    assert_eq!(run.instances["a"].status, StepStatus::Ready);
    assert_eq!(run.instances["b"].status, StepStatus::Ready);
    assert_eq!(run.instances["c"].status, StepStatus::Waiting);
    prepare(&mut run, "a", "root-node", 2).unwrap();
    prepare(&mut run, "b", "root-node", 2).unwrap();
    let event = notice(&run, "a", json!("a-output"));
    apply_notice(&mut run, &event).unwrap();
    assert_eq!(run.instances["c"].status, StepStatus::Ready);
    assert_eq!(run.instances["b"].status, StepStatus::Queued);
    assert_eq!(run.instances["c"].inputs["text"], "a-output");
    let revision = run.revision;
    assert!(!apply_notice(&mut run, &event).unwrap());
    assert_eq!(run.revision, revision);
}

#[test]
fn plan_input_is_persistent_validated_and_immutable_after_supply() {
    let mut first = step("a");
    first["inputs"] =
        json!({"name":{"schema":{"type":"string"},"binding":{"source":"input","name":"name"}}});
    let mut p = plan(json!([first]), json!({"source":"output","step":"a"}));
    p.inputs = serde_json::from_value(
        json!({"name":{"description":"User provides a name","schema":{"type":"string"}}}),
    )
    .unwrap();
    let mut run = run(p);
    assert_eq!(run.phase, RunPhase::WaitingInput);
    assert!(supply_input(&mut run, "name", json!(7), 2).is_err());
    supply_input(&mut run, "name", json!("Ada"), 2).unwrap();
    assert_eq!(run.phase, RunPhase::Running);
    assert!(supply_input(&mut run, "name", json!("Bob"), 3).is_err());
}

#[test]
fn pause_fences_old_decisions_and_cancel_waits_for_child_receipts() {
    let mut run = run(plan(
        json!([step("a"), step("b")]),
        json!({"source":"output","step":"a"}),
    ));
    let context = context(&mut run);
    prepare(&mut run, "a", "root-node", 2).unwrap();
    command(&mut run, "pause", 2).unwrap();
    let decision = ActivationDecision {
        run_id: run.id.clone(),
        activation: context.activation,
        control_epoch: context.control_epoch,
        reason: "dispatch".into(),
        plan: None,
        dispatch: vec!["b".into()],
    };
    assert!(decide(&run, &decision).is_err());
    assert!(prepare(&mut run, "b", "root-node", 2).is_err());
    command(&mut run, "cancel", 3).unwrap();
    assert_eq!(run.phase, RunPhase::Cancelling);
    let mut event = notice(&run, "a", json!("done"));
    event.status = StepStatus::Cancelled;
    event.output = None;
    apply_notice(&mut run, &event).unwrap();
    assert_eq!(run.phase, RunPhase::Cancelled);
}

#[test]
fn output_contract_failure_is_visible_and_independent_work_continues() {
    let mut run = run(plan(
        json!([step("a"), step("b")]),
        json!({"source":"output","step":"a"}),
    ));
    prepare(&mut run, "a", "root-node", 2).unwrap();
    prepare(&mut run, "b", "root-node", 2).unwrap();
    let event = notice(&run, "a", json!(17));
    apply_notice(&mut run, &event).unwrap();
    assert_eq!(run.instances["a"].status, StepStatus::Failed);
    assert_eq!(run.phase, RunPhase::Running);
    let event = notice(&run, "b", json!("ok"));
    apply_notice(&mut run, &event).unwrap();
    assert_eq!(run.phase, RunPhase::Failed);
}

#[test]
fn sealed_expansion_skips_false_branches_and_joins_all_outputs() {
    let mut each = step("each");
    each["foreach"] = json!({"items":{"source":"input","name":"items"},"key":"/id"});
    each["when"] = json!({"value":{"source":"item","path":"/enabled"},"equals":true});
    each["inputs"] =
        json!({"id":{"schema":{"type":"string"},"binding":{"source":"item","path":"/id"}}});
    let mut join = step("join");
    join["inputs"] = json!({"all":{"schema":{"type":"array","items":{"type":"string"}},"binding":{"source":"output","step":"each","collect":true}}});
    let mut p = plan(
        json!([each, join]),
        json!({"source":"output","step":"join"}),
    );
    p.inputs=serde_json::from_value(json!({"items":{"description":"Batch","schema":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"enabled":{"type":"boolean"}},"required":["id","enabled"]}}}})).unwrap();
    let mut run = run(p);
    supply_input(
        &mut run,
        "items",
        json!([{"id":"x","enabled":true},{"id":"y","enabled":false}]),
        2,
    )
    .unwrap();
    assert_eq!(run.expansions["each"].len(), 2);
    let first = run
        .instances
        .values()
        .find(|i| i.step_id == "each" && i.status == StepStatus::Ready)
        .unwrap()
        .id
        .clone();
    assert!(run
        .instances
        .values()
        .any(|i| i.status == StepStatus::Skipped));
    prepare(&mut run, &first, "root-node", 2).unwrap();
    let event = notice(&run, &first, json!("x-done"));
    apply_notice(&mut run, &event).unwrap();
    assert_eq!(run.instances["join"].status, StepStatus::Ready);
    assert_eq!(run.instances["join"].inputs["all"], json!(["x-done"]));
    prepare(&mut run, "join", "root-node", 3).unwrap();
    let event = notice(&run, "join", json!("joined"));
    apply_notice(&mut run, &event).unwrap();
    assert_eq!(run.phase, RunPhase::Completed);
    assert_eq!(run.request.plan.as_ref().unwrap().version, 1);
    let version = run.request.plan.clone().unwrap();
    assert!(adopt(&mut run, version, 5).is_err());
}

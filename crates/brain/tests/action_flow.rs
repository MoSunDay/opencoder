mod support;
use opencoder_brain::{execution::*, ontology::validate};
use opencoder_core::brain::*;
use serde_json::json;
use support::*;

fn repair_plan() -> OntologyPlan {
    serde_json::from_str(include_str!("../../../examples/brain/repair-loop.json")).unwrap()
}
fn start() -> BrainRun {
    let mut state = run(repair_plan());
    assert_eq!(state.phase, RunPhase::WaitingInput);
    supply_input(&mut state, "problem", json!("修复复现问题"), 2).unwrap();
    state
}
fn finish(state: &mut BrainRun, value: serde_json::Value) -> BrainNotice {
    let id = state.flow_current.clone().unwrap();
    prepare(state, &id, "root-node", 3).unwrap();
    let event = notice(state, &id, value);
    apply_notice(state, &event).unwrap();
    event
}

#[test]
fn repair_retest_return_and_release_use_new_visits_and_latest_feedback() {
    let mut state = start();
    assert!(!state.instances[state.flow_current.as_ref().unwrap()]
        .inputs
        .contains_key("feedback"));
    finish(&mut state, json!({"commit":"fix-1"}));
    let old = finish(&mut state, json!({"passed":false,"issues":"still broken"}));
    let fix2 = state.flow_current.clone().unwrap();
    assert_eq!(state.instances[&fix2].step_id, "fix");
    assert_eq!(state.instances[&fix2].inputs["feedback"], "still broken");
    assert!(!state.instances.values().any(|i| i.step_id == "release"));
    // Restart from persisted state and replay a prior notice: no extra visit.
    state = serde_json::from_value(serde_json::to_value(state).unwrap()).unwrap();
    assert!(!apply_notice(&mut state, &old).unwrap());
    assert_eq!(state.flow_current.as_deref(), Some(fix2.as_str()));
    finish(&mut state, json!({"commit":"fix-2"}));
    finish(&mut state, json!({"passed":true,"issues":""}));
    let release = &state.instances[state.flow_current.as_ref().unwrap()];
    assert_eq!(release.step_id, "release");
    assert_eq!(release.inputs["commit"], "fix-2");
    assert_eq!(release.inputs["verified"], true);
    finish(&mut state, json!({"result":"released fix-2"}));
    assert_eq!(state.phase, RunPhase::Completed);
    assert_eq!(state.deliverables["release_result"], "released fix-2");
    assert_eq!(state.instances.len(), 5);
    assert_eq!(state.actions.len(), 5);
    assert!(state
        .instances
        .values()
        .all(|i| i.status == StepStatus::Succeeded));
}

#[test]
fn visit_budget_and_malformed_verification_never_release() {
    let mut plan = repair_plan();
    plan.flow.as_mut().unwrap().max_visits_per_action = 1;
    let mut state = run(plan);
    supply_input(&mut state, "problem", json!("broken"), 2).unwrap();
    finish(&mut state, json!({"commit":"fix-1"}));
    finish(&mut state, json!({"passed":false,"issues":"broken"}));
    assert_eq!(state.phase, RunPhase::Failed);
    assert!(state.error.unwrap().contains("visit limit"));
    assert!(!state.instances.values().any(|i| i.step_id == "release"));
    let mut state = start();
    finish(&mut state, json!({"commit":"fix-1"}));
    finish(&mut state, json!({"passed":"yes","issues":""}));
    assert_eq!(state.phase, RunPhase::Failed);
    assert!(!state.instances.values().any(|i| i.step_id == "release"));
}

#[test]
fn pause_fences_transition_and_cancel_prevents_release() {
    let mut state = start();
    let id = state.flow_current.clone().unwrap();
    prepare(&mut state, &id, "root-node", 3).unwrap();
    command(&mut state, "pause", 4).unwrap();
    let event = notice(&state, &id, json!({"commit":"fix-1"}));
    apply_notice(&mut state, &event).unwrap();
    assert_eq!(state.phase, RunPhase::Paused);
    assert_eq!(state.instances.len(), 1);
    command(&mut state, "resume", 5).unwrap();
    assert_eq!(
        state.instances[state.flow_current.as_ref().unwrap()].step_id,
        "verify"
    );
    command(&mut state, "cancel", 6).unwrap();
    opencoder_brain::execution::advance(&mut state, 7).unwrap();
    assert_eq!(state.phase, RunPhase::Cancelled);
    assert!(!state.instances.values().any(|i| i.step_id == "release"));
}

#[test]
fn invalid_routes_and_ambiguous_conditions_are_rejected() {
    let valid = repair_plan();
    validate(&valid).unwrap();
    let mut plan = valid.clone();
    plan.flow.as_mut().unwrap().transitions[0].to = Some("absent".into());
    assert!(validate(&plan).unwrap_err().to_string().contains("target"));
    let mut plan = valid.clone();
    plan.flow.as_mut().unwrap().transitions[2]
        .when
        .as_mut()
        .unwrap()
        .equals = json!(false);
    assert!(validate(&plan)
        .unwrap_err()
        .to_string()
        .contains("duplicate"));
    let mut plan = valid.clone();
    plan.flow.as_mut().unwrap().transitions[3].to = Some("fix".into());
    assert!(validate(&plan).unwrap_err().to_string().contains("finish"));
    let mut plan = valid;
    plan.flow = None;
    assert!(validate(&plan).unwrap_err().to_string().contains("cycle"));
}

#[test]
fn cancellation_waits_for_active_receipt_and_never_follows_success_edge() {
    let mut state = start();
    let id = state.flow_current.clone().unwrap();
    prepare(&mut state, &id, "root-node", 3).unwrap();
    command(&mut state, "cancel", 4).unwrap();
    assert_eq!(state.phase, RunPhase::Cancelling);
    assert!(state.instances[&id].status.active());
    let event = notice(&state, &id, json!({"commit":"fix-1"}));
    apply_notice(&mut state, &event).unwrap();
    assert_eq!(state.phase, RunPhase::Cancelled);
    assert_eq!(state.instances.len(), 1);
}

#[test]
fn optional_user_input_can_be_absent_but_required_input_remains_a_durable_request() {
    let mut state = run(repair_plan());
    assert_eq!(state.phase, RunPhase::WaitingInput);
    assert!(!state.input_requests["problem"].answered);
    assert!(supply_input(&mut state, "problem", json!(12), 2).is_err());
    state = serde_json::from_value(serde_json::to_value(state).unwrap()).unwrap();
    supply_input(&mut state, "problem", json!("fixed repro"), 3).unwrap();
    assert!(state.input_requests["problem"].answered);
    assert_eq!(state.phase, RunPhase::Running);

    let mut plan = repair_plan();
    plan.inputs.get_mut("problem").unwrap().required = false;
    plan.steps[0].inputs.get_mut("problem").unwrap().required = false;
    let state = run(plan);
    assert_eq!(state.phase, RunPhase::Running);
    assert!(state.instances[state.flow_current.as_ref().unwrap()]
        .inputs
        .is_empty());
}

#[test]
fn self_loop_reads_previous_visit_until_new_inputs_are_bound() {
    let mut plan = repair_plan();
    plan.inputs.clear();
    plan.steps.retain(|s| s.id == "verify");
    plan.steps[0].inputs = serde_json::from_value(json!({"feedback":{
        "schema":{"type":"string"},"binding":{"source":"output","step":"verify","path":"/issues"},"required":false
    }})).unwrap();
    plan.deliverables = serde_json::from_value(json!({"passed":{
        "description":"verified", "source":{"source":"output","step":"verify","path":"/passed"}, "schema":{"type":"boolean"},"expected":true
    }})).unwrap();
    plan.flow = Some(serde_json::from_value(json!({"entry":"verify","max_visits_per_action":3,"transitions":[
        {"from":"verify","to":"verify","label":"retry","when":{"value":{"source":"output","step":"verify","path":"/passed"},"equals":false}},
        {"from":"verify","to":null,"label":"done","when":{"value":{"source":"output","step":"verify","path":"/passed"},"equals":true}}
    ]})).unwrap());
    let mut state = run(plan);
    finish(&mut state, json!({"passed":false,"issues":"retry this"}));
    assert_eq!(
        state.instances[state.flow_current.as_ref().unwrap()].inputs["feedback"],
        "retry this"
    );
    finish(&mut state, json!({"passed":true,"issues":""}));
    assert_eq!(state.phase, RunPhase::Completed);
    assert_eq!(state.instances.len(), 2);
}

#[test]
fn unavailable_ambiguous_and_unmatched_conditions_fail_before_release() {
    for case in ["missing", "ambiguous", "unmatched"] {
        let mut plan = repair_plan();
        let flow = plan.flow.as_mut().unwrap();
        if case == "missing" {
            // A default release edge must never hide missing verifier evidence.
            plan.steps[1]
                .output
                .required
                .retain(|name| name != "passed");
            flow.transitions[2].when = None;
        } else if case == "ambiguous" {
            flow.transitions[2].when = Some(
                serde_json::from_value(json!({
                    "value":{"source":"output","step":"verify","path":"/issues"},"equals":"bug"
                }))
                .unwrap(),
            );
        } else {
            flow.transitions.remove(1);
        }
        let mut state = run(plan);
        supply_input(&mut state, "problem", json!("repro"), 2).unwrap();
        finish(&mut state, json!({"commit":"fix-1"}));
        finish(
            &mut state,
            if case == "missing" {
                json!({"issues":"bug"})
            } else {
                json!({"passed":false,"issues":"bug"})
            },
        );
        assert_eq!(state.phase, RunPhase::Failed, "{case}");
        assert!(
            !state.instances.values().any(|i| i.step_id == "release"),
            "{case}"
        );
    }
}

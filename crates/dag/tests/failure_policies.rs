use opencoder_dag::{decode_spec, ready_steps, StepOutcome, StepStates};
use serde_json::json;

#[test]
fn policies_survive_registration_serialization_and_wait_for_terminal_dependencies() {
    let value = json!({"name":"native", "steps":[
        {"name":"cases","kind":{"type":"dynamic","failure_policy":"collect_all",
         "source":{"type":"input","pointer":"/items"},"template":{"type":"agent","prompt":"test"}}},
        {"name":"summary","depends_on":["cases"],"trigger_rule":"all_done","kind":{"type":"agent","prompt":"summary"}}
    ]});
    let spec = decode_spec(&value).unwrap();
    let saved = serde_json::to_value(&spec).unwrap();
    assert_eq!(saved["steps"][0]["kind"]["failure_policy"], "collect_all");
    assert_eq!(saved["steps"][1]["trigger_rule"], "all_done");
    assert_eq!(ready_steps(&spec, &StepStates::new()), vec!["cases"]);
    for outcome in [StepOutcome::Done, StepOutcome::Error, StepOutcome::Cancelled] {
        let states = [("cases".into(), outcome)].into_iter().collect();
        assert_eq!(ready_steps(&spec, &states), vec!["summary"]);
    }
}

#[test]
fn invalid_policies_are_rejected_instead_of_discarded() {
    let mut value = json!({"name":"native", "steps":[
        {"name":"cases","trigger_rule":"typo","kind":{"type":"agent","prompt":"test"}}
    ]});
    assert!(decode_spec(&value).is_err());
    value["steps"][0]["trigger_rule"] = json!("all_success");
    value["steps"][0]["kind"] = json!({"type":"dynamic","failure_policy":"typo",
        "source":{"type":"input","pointer":"/items"},"template":{"type":"agent","prompt":"test"}});
    assert!(decode_spec(&value).is_err());
}

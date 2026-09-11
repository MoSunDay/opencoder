mod support;
use opencoder_brain::ontology::*;
use serde_json::json;
use support::*;
#[test]
fn rejects_cycles_unknown_ports_wrong_types_and_invalid_semantics() {
    let mut a = step("a");
    let mut b = step("b");
    a["depends_on"] = json!(["b"]);
    b["depends_on"] = json!(["a"]);
    assert!(
        validate(&plan(json!([a, b]), json!({"source":"output","step":"a"})))
            .unwrap_err()
            .to_string()
            .contains("cycle")
    );
    let mut b = step("b");
    b["inputs"] =
        json!({"x":{"schema":{"type":"integer"},"binding":{"source":"output","step":"a"}}});
    assert!(validate(&plan(
        json!([step("a"), b]),
        json!({"source":"output","step":"a"})
    ))
    .is_err());
    let mut a = step("a");
    a["inputs"] =
        json!({"x":{"schema":{"type":"string"},"binding":{"source":"input","name":"missing"}}});
    assert!(validate(&plan(json!([a]), json!({"source":"output","step":"a"}))).is_err());
}
#[test]
fn empty_plan_or_missing_deliverables_cannot_report_success() {
    let mut p = plan(json!([]), json!({"source":"literal","value":"x"}));
    assert!(validate(&p).is_err());
    p.steps = vec![serde_json::from_value(step("a")).unwrap()];
    p.deliverables.clear();
    assert!(validate(&p).is_err());
}

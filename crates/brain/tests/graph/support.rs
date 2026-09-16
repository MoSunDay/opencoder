#![allow(dead_code)]
use opencoder_brain::{execution as exec, graph};
use opencoder_core::brain::*;
use serde_json::{json, Value};
pub fn fixture() -> OntologyPlan {
    serde_json::from_str(include_str!("../../../../examples/brain/repair-loop.json")).unwrap()
}
pub fn version(plan: OntologyPlan) -> PlanVersion {
    serde_json::from_value(
        json!({"id":"plan-test","version":1,"plan":plan,"created_at":1,"changelog":"fixture"}),
    )
    .unwrap()
}
pub fn run(plan: OntologyPlan) -> BrainRun {
    exec::initialize("brain-test", serde_json::from_value(json!({"schema_version":2,"mode":"fixed","objective":"fixture","plan":version(plan),"inputs":{"document":{"name":"Requirement","markdown":"Fix the issue"}}})).unwrap(), 1).unwrap()
}
pub fn visit(run: &BrainRun, step: &str) -> String {
    run.expansions[step].last().unwrap().clone()
}
pub fn output(content: &str, passed: Option<bool>) -> Value {
    json!({"content":content,"completion":{"passed":true,"evidence":["delivery exists"]},"verification":{"passed":passed,"evidence":if passed.is_some(){vec!["test evidence"]}else{vec![]}}})
}
pub fn notice(run: &BrainRun, id: &str, value: Value) -> BrainNotice {
    let instance = &run.instances[id];
    BrainNotice {
        parent: BrainLink {
            run_id: run.id.clone(),
            node_id: "root".into(),
            instance_id: id.into(),
            attempt: instance.attempt,
        },
        execution: instance.execution.clone().unwrap(),
        node_id: "child".into(),
        sequence: 3,
        status: StepStatus::Succeeded,
        output: Some(OutputEnvelope {
            value,
            artifacts: vec![],
            evidence: vec![],
        }),
        error: None,
        at_ms: 3,
    }
}
pub fn finish(run: &mut BrainRun, step: &str, values: Value) -> BrainNotice {
    let id = visit(run, step);
    exec::prepare(run, &id, "root", 2).unwrap();
    let n = notice(run, &id, values);
    exec::apply_notice(run, &n).unwrap();
    n
}
pub fn choose(
    run: &mut BrainRun,
    route: &str,
    selected: &[&str],
    exit: Option<&str>,
) -> RouteDecision {
    let context = graph::pending(run)
        .into_iter()
        .find(|c| c.route == route)
        .unwrap();
    let d = RouteDecision {
        receipt: context.receipt,
        reason: "output evidence justifies this choice".into(),
        selected: selected.iter().map(|s| (*s).into()).collect(),
        exit: exit.map(str::to_owned),
        blocked: None,
    };
    graph::apply(run, &d, 4).unwrap();
    d
}

#![allow(dead_code)]
use opencoder_core::brain::*;
use serde_json::{json, Value};
pub fn plan(steps: Value, source: Value) -> OntologyPlan {
    serde_json::from_value(json!({"schema_version":1,"title":"test","objective":"test parallel execution","steps":steps,"deliverables":{"result":{"description":"result","source":source,"schema":{"type":"string"}}}})).unwrap()
}
pub fn step(id: &str) -> Value {
    json!({"id":id,"label":id,"purpose":"Produce evidence","action":{"kind":"agent","target":"act","prompt":"Return verified result"},"inputs":{},"output":{"type":"string"},"acceptance":"correct result"})
}
pub fn version(plan: OntologyPlan) -> PlanVersion {
    PlanVersion {
        id: "plan-test".into(),
        version: 1,
        plan,
        changelog: "initial".into(),
        tags: vec![],
        confidence: Confidence::default(),
        created_at: 1,
        author: "test".into(),
    }
}
pub fn run(plan: OntologyPlan) -> BrainRun {
    opencoder_brain::execution::initialize(
        "brain-test",
        BrainRequest {
            mode: PlanningMode::Fixed,
            objective: plan.objective.clone(),
            plan: Some(version(plan)),
            inputs: Default::default(),
            references: vec![],
            capabilities: vec![],
        },
        1,
    )
    .unwrap()
}
pub fn notice(run: &BrainRun, id: &str, value: Value) -> BrainNotice {
    let instance = &run.instances[id];
    BrainNotice {
        parent: BrainLink {
            run_id: run.id.clone(),
            node_id: "root-node".into(),
            instance_id: id.into(),
            attempt: instance.attempt,
        },
        execution: instance.execution.clone().unwrap(),
        node_id: "child-node".into(),
        sequence: 3,
        status: StepStatus::Succeeded,
        output: Some(OutputEnvelope {
            value,
            artifacts: vec![],
            evidence: vec!["verified".into()],
        }),
        error: None,
        at_ms: 4,
    }
}

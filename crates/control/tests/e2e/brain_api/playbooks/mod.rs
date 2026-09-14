//! Playbook control-plane surface: list/get passthrough, dispatch expanding
//! a spec into topological batches (request-scoped idempotency), brain-step
//! target binding resolution, and the 一期 manual trigger scan.

mod dispatch;
mod trigger;

use opencoder_brain::{
    playbook, PlaybookOrigin, PlaybookSpec, PlaybookStep, PlaybookTarget, PlaybookTrigger,
};
use opencoder_core::fleet::ExecutionKind;
use opencoder_store::BrainPlaybookRecord;
use reqwest::Method;
use serde_json::{json, Value};

use crate::support::Harness;

const DAG_SPEC: &str = r#"{"name":"pbk-dag","steps":[
    {"name":"fetch","kind":{"type":"wasm","command":"tool.wasm"}},
    {"name":"load","depends_on":["fetch"],"kind":{"type":"wasm","command":"tool.wasm"}}]}"#;

pub(crate) fn step(name: &str, deps: &[&str], target: PlaybookTarget) -> PlaybookStep {
    PlaybookStep {
        name: name.into(),
        depends_on: deps.iter().map(|dep| dep.to_string()).collect(),
        target,
        prompt: format!("handle {{situation}} for {name}"),
    }
}

pub(crate) fn agent(name: &str) -> PlaybookStep {
    step(
        name,
        &[],
        PlaybookTarget::Agent {
            agent: "act".into(),
        },
    )
}

pub(crate) fn spec(id: &str, trigger: PlaybookTrigger, steps: Vec<PlaybookStep>) -> PlaybookSpec {
    PlaybookSpec {
        schema_version: playbook::SCHEMA_VERSION,
        id: id.into(),
        name: id.into(),
        origin: PlaybookOrigin::Fixed {},
        trigger,
        steps,
    }
}

pub(crate) async fn seed_playbook(h: &Harness, spec: &PlaybookSpec) {
    let now = opencoder_core::message::now_ms();
    h.state
        .store
        .save_brain_playbook(&BrainPlaybookRecord {
            id: spec.id.clone(),
            name: spec.name.clone(),
            origin: "fixed".into(),
            situation_digest: None,
            spec_json: serde_json::to_string(spec).unwrap(),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
}

pub(crate) async fn seed_dag(h: &Harness) {
    let (status, body) = h
        .req(
            Method::POST,
            "/api/dag/defs",
            Some(json!({"spec": serde_json::from_str::<Value>(DAG_SPEC).unwrap()})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
}

pub(crate) async fn index_kind(h: &Harness, id: &str) -> Option<ExecutionKind> {
    h.state.fleet.index(id).await.unwrap().map(|i| i.kind)
}

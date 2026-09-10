//! Dynamic-playbook planner contract tests: the second scheduling track
//! (`plan_playbook`) over the same deterministic mocks `tests/planning.rs`
//! uses — the mock embedder is a pure hash, so retrieval returns both
//! seeded capabilities for every situation; the mock chat client scripts
//! each planner reply. Covers the digest cache, the empty-library default,
//! and the typed failure paths for contract violations.

use std::sync::Arc;

use opencoder_brain::playbook::{PlaybookOrigin, PlaybookTarget, PlaybookTrigger};
use opencoder_brain::{situation_digest, CapabilityInput, PlanGenerationFailed, Runtime};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, Store};
use serde_json::json;

const MODEL: &str = "mock-embed-v1";
const SITUATION_A: &str = "release blockers piled up before the friday cut";
const SITUATION_B: &str = "a stranger repo needs an onboarding tour";

async fn setup() -> (Runtime, Arc<MockChatClient>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(MockChatClient::new());
    let client: Arc<dyn ChatStream> = mock.clone();
    (Runtime::new(store, client, MODEL), mock)
}

fn capability(summary: &str) -> CapabilityInput {
    CapabilityInput {
        capability_type: "tool-usage".into(),
        summary: summary.into(),
        input_desc: "a work request".into(),
        output_desc: "completed work".into(),
        eng_inputs: vec!["exemplar input".into()],
    }
}

async fn seeded() -> (Runtime, Arc<MockChatClient>, [String; 2]) {
    let (rt, mock) = setup().await;
    let a = rt
        .upsert_capability(&capability("db migration plan"), 1_000)
        .await
        .unwrap();
    let b = rt
        .upsert_capability(&capability("write unit tests"), 1_100)
        .await
        .unwrap();
    (rt, mock, [a.capability.id, b.capability.id])
}

/// Queue one scripted planner reply (deltas + completed), fenced to prove
/// parse robustness — the same discipline `queue_tree` applies.
fn queue_reply(mock: &MockChatClient, body: serde_json::Value) {
    let reply = format!("```json\n{}\n```", body);
    mock.queue_script(vec![
        LlmEvent::TextDelta(reply.clone()),
        LlmEvent::Completed {
            text: reply,
            tool_calls: Vec::new(),
            usage: None,
        },
    ]);
}

fn step(name: &str, deps: &[&str], target: PlaybookTarget, prompt: &str) -> serde_json::Value {
    json!({
        "name": name,
        "depends_on": deps,
        "target": target,
        "prompt": prompt,
    })
}

fn brain(capability_id: &str) -> PlaybookTarget {
    PlaybookTarget::Brain {
        capability_id: capability_id.into(),
    }
}

#[tokio::test]
async fn plan_persists_and_roundtrips() {
    let (rt, mock, caps) = seeded().await;
    // Diamond: fetch → (review, audit) → merge; one brain step on candidate
    // A, one agent step, prompts templated on {situation}, message trigger.
    queue_reply(
        &mock,
        json!({
            "name": "cut-release-playbook",
            "trigger": {"kind": "message", "match_text": "release", "threshold": 0.82},
            "steps": [
                step("fetch", &[], brain(&caps[0]), "调研 {situation}"),
                step("review", &["fetch"],
                     PlaybookTarget::Agent { agent: "reviewer".into() }, "评审 {situation}"),
                step("audit", &["fetch"],
                     PlaybookTarget::Team { team: "auditors".into() }, "审计 {situation}"),
                step("merge", &["review", "audit"],
                     PlaybookTarget::Dag { dag: "merge-flow".into() }, "合并 {situation}"),
            ],
        }),
    );

    let planned = rt
        .plan_playbook("planner-chat", SITUATION_A, 5, 2_000)
        .await
        .unwrap();
    assert!(planned.planned_fresh);
    assert_eq!(planned.record.origin, "dynamic");
    assert_eq!(
        planned.record.situation_digest.as_deref(),
        Some(situation_digest(SITUATION_A).as_str())
    );
    assert!(
        planned.record.id.starts_with("playbook-"),
        "{}",
        planned.record.id
    );
    assert_eq!(planned.spec.id, planned.record.id);
    assert_eq!(planned.spec.name, "cut-release-playbook");
    assert!(matches!(
        &planned.spec.origin,
        PlaybookOrigin::Dynamic { situation_digest, plan_id: None }
            if situation_digest == &planned.record.situation_digest.clone().unwrap()
    ));
    assert!(matches!(
        &planned.spec.trigger,
        PlaybookTrigger::Message { match_text, threshold }
            if match_text == "release" && (*threshold - 0.82).abs() < 1e-9
    ));
    assert_eq!(planned.spec.steps.len(), 4);
    assert_eq!(planned.spec.steps[0].target, brain(&caps[0]));

    // The spec round-trips through the store byte-identically.
    let stored = rt
        .get_playbook_spec(&planned.record.id)
        .await
        .unwrap()
        .expect("persisted");
    assert_eq!(stored, planned.spec);

    // Same situation again with NOTHING queued: the digest cache answers,
    // no LLM call is made, the same playbook id is reused.
    let calls = mock.call_count();
    assert_eq!(calls, 1, "exactly one planner call so far");
    let cached = rt
        .plan_playbook("planner-chat", SITUATION_A, 5, 3_000)
        .await
        .unwrap();
    assert!(!cached.planned_fresh);
    assert_eq!(cached.record.id, planned.record.id);
    assert_eq!(cached.spec, planned.spec);
    assert_eq!(mock.call_count(), calls, "cache hit must not touch the LLM");
}

#[tokio::test]
async fn empty_library_returns_default_act_playbook() {
    let (rt, mock) = setup().await; // no capabilities, no scripts queued
    let planned = rt
        .plan_playbook("planner-chat", SITUATION_A, 5, 2_000)
        .await
        .unwrap();
    assert!(planned.planned_fresh);
    assert_eq!(mock.call_count(), 0, "empty library must not call the LLM");
    assert_eq!(planned.spec.name, "default-act");
    assert!(matches!(planned.spec.trigger, PlaybookTrigger::Manual {}));
    assert_eq!(planned.spec.steps.len(), 1);
    let step = &planned.spec.steps[0];
    assert_eq!(step.name, "act");
    assert_eq!(
        step.target,
        PlaybookTarget::Agent {
            agent: "act".into()
        }
    );
    // Raw template keeps the {situation} placeholder verbatim.
    assert_eq!(step.prompt, "{situation}");
    assert_eq!(planned.record.origin, "dynamic");
    // Persisted like any planner mint.
    let stored = rt
        .get_playbook_spec(&planned.record.id)
        .await
        .unwrap()
        .expect("persisted");
    assert_eq!(stored, planned.spec);
}

#[tokio::test]
async fn contract_violation_is_typed_generation_failure() {
    let (rt, mock, _caps) = seeded().await;

    // A brain step targeting an id OUTSIDE the retrieved candidate set.
    queue_reply(
        &mock,
        json!({
            "name": "fabricated",
            "steps": [step("fetch", &[], brain("brain-fabricated"), "处理 {situation}")],
        }),
    );
    let err = rt
        .plan_playbook("planner-chat", SITUATION_A, 5, 2_000)
        .await
        .unwrap_err();
    let typed = err
        .downcast_ref::<PlanGenerationFailed>()
        .expect("typed PlanGenerationFailed marker");
    assert!(
        typed.detail.contains("brain-fabricated") && typed.detail.contains("not a candidate"),
        "detail must name the invalid target: {typed:?}"
    );
    // Nothing invalid is persisted.
    assert!(rt.list_playbooks().await.unwrap().is_empty());

    // A cyclic depends_on graph.
    queue_reply(
        &mock,
        json!({
            "name": "cyclic",
            "steps": [
                step("a", &["b"], brain("anything-at-all"), "处理 {situation}"),
                step("b", &["a"], brain("anything-at-all"), "处理 {situation}"),
            ],
        }),
    );
    let err = rt
        .plan_playbook("planner-chat", SITUATION_A, 5, 2_000)
        .await
        .unwrap_err();
    let typed = err
        .downcast_ref::<PlanGenerationFailed>()
        .expect("typed PlanGenerationFailed marker");
    assert!(
        typed.detail.contains("cycle"),
        "detail must name the cycle: {typed:?}"
    );
}

#[tokio::test]
async fn unparseable_reply_fails() {
    let (rt, mock, _caps) = seeded().await;
    let reply = "I cannot produce JSON today, sorry.".to_string();
    mock.queue_script(vec![
        LlmEvent::TextDelta(reply.clone()),
        LlmEvent::Completed {
            text: reply,
            tool_calls: Vec::new(),
            usage: None,
        },
    ]);
    let err = rt
        .plan_playbook("planner-chat", SITUATION_A, 5, 2_000)
        .await
        .unwrap_err();
    assert!(
        err.downcast_ref::<PlanGenerationFailed>().is_some(),
        "unparseable reply must be a typed failure, got: {err:#}"
    );
}

#[tokio::test]
async fn distinct_situations_do_not_share_the_cache() {
    let (rt, mock, caps) = seeded().await;
    assert_ne!(
        situation_digest(SITUATION_A),
        situation_digest(SITUATION_B),
        "fixtures must have distinct digests"
    );
    queue_reply(
        &mock,
        json!({"name": "one", "steps": [step("act", &[], brain(&caps[0]), "{situation}")]}),
    );
    queue_reply(
        &mock,
        json!({"name": "two", "steps": [step("act", &[], brain(&caps[1]), "{situation}")]}),
    );

    let a = rt
        .plan_playbook("planner-chat", SITUATION_A, 5, 2_000)
        .await
        .unwrap();
    let b = rt
        .plan_playbook("planner-chat", SITUATION_B, 5, 2_100)
        .await
        .unwrap();
    assert!(a.planned_fresh && b.planned_fresh, "both must plan fresh");
    assert_ne!(a.record.id, b.record.id);
    assert_ne!(a.record.situation_digest, b.record.situation_digest);
    assert_eq!(mock.call_count(), 2, "one planner call per situation");
}

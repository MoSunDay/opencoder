//! Dynamic-planner contract tests: framework-prompt planning over the mock
//! embedder (a pure hash — identical texts embed identically, cosine 1.0;
//! distinct texts land ≤ ~0.95), so a threshold of 0.98 makes branch walks
//! fully deterministic: the situation text EQUALS a branch topic to take the
//! yes edge, anything else takes no.

use std::sync::Arc;

use opencoder_brain::plan::{self, PlanNode};
use opencoder_brain::{
    CapabilityInput, DispatchOutcome, PlanGenerationFailed, PlanNotFound, Runtime,
};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, Store};

const MODEL: &str = "mock-embed-v1";
const TOPIC_A: &str = "db migration plan";
const TOPIC_B: &str = "write unit tests";
const THRESH: f64 = 0.98;

async fn setup() -> (Runtime, Arc<MockChatClient>, Arc<dyn Store>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mock = Arc::new(MockChatClient::new());
    let client: Arc<dyn ChatStream> = mock.clone();
    (Runtime::new(store.clone(), client, MODEL), mock, store)
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

async fn seeded() -> (Runtime, Arc<MockChatClient>, Arc<dyn Store>, [String; 2]) {
    let (rt, mock, store) = setup().await;
    let a = rt
        .upsert_capability(&capability(TOPIC_A), 1_000)
        .await
        .unwrap();
    let b = rt
        .upsert_capability(&capability(TOPIC_B), 1_100)
        .await
        .unwrap();
    (rt, mock, store, [a.capability.id, b.capability.id])
}

/// The scripted planner reply: a fenced JSON tree routing topic A → cap A
/// (yes) and everything else → cap B (no). Fences prove parse robustness.
fn tree_reply(cap_a: &str, cap_b: &str) -> String {
    format!(
        "```json\n{{\"threshold\":{THRESH},\"root\":{{\"id\":\"b1\",\"kind\":\"branch\",\"topic\":\"{TOPIC_A}\",\"reason\":\"route db work\",\"yes\":{{\"id\":\"l1\",\"kind\":\"leaf\",\"capability_id\":\"{cap_a}\",\"reason\":\"db work goes to the migration capability\"}},\"no\":{{\"id\":\"l2\",\"kind\":\"leaf\",\"capability_id\":\"{cap_b}\",\"reason\":\"everything else is test work\"}}}}}}\n```"
    )
}

fn queue_tree(mock: &MockChatClient, reply: String) {
    mock.queue_script(vec![
        LlmEvent::TextDelta(reply.clone()),
        LlmEvent::Completed {
            text: reply,
            tool_calls: Vec::new(),
            usage: None,
        },
    ]);
}

#[tokio::test]
async fn plan_persists_tree_and_dispatch_routes_by_topic() {
    let (rt, mock, store, caps) = seeded().await;
    queue_tree(&mock, tree_reply(&caps[0], &caps[1]));

    let (record, tree) = rt
        .plan_decision_tree("planner-chat", TOPIC_A, 5, 2_000)
        .await
        .unwrap();
    assert!(record.id.starts_with("brain-plan-"), "{}", record.id);
    assert_eq!(record.chat_model, "planner-chat");
    assert_eq!(record.situation, TOPIC_A);
    // The plan round-trips through the store with its vectors attached.
    let stored = store
        .get_brain_plan(&record.id)
        .await
        .unwrap()
        .expect("plan must persist");
    assert_eq!(stored.id, record.id);
    let PlanNode::Branch { topic_vec, .. } = &tree.root else {
        panic!("root must be a branch");
    };
    let vec = topic_vec.as_ref().expect("topic vector attached");
    let norm: f64 = vec
        .iter()
        .map(|c| (*c as f64) * (*c as f64))
        .sum::<f64>()
        .sqrt();
    assert!(
        (norm - 1.0).abs() < 1e-5,
        "topic vector must be unit-norm: {norm}"
    );

    // Situation EQUALS the branch topic → cosine 1.0 ≥ 0.98 → yes → cap A.
    let (_, hit) = rt
        .dispatch_decision_tree(&record.id, TOPIC_A)
        .await
        .unwrap();
    assert_eq!(hit.capability_id, caps[0]);
    assert_eq!(hit.path.len(), 2, "branch + leaf steps");
    assert_eq!(hit.path[0].taken, Some(true));

    // Any other text lands below threshold → no → cap B.
    let (_, miss) = rt
        .dispatch_decision_tree(&record.id, "an unrelated production incident")
        .await
        .unwrap();
    assert_eq!(miss.capability_id, caps[1]);
    assert_eq!(miss.path[0].taken, Some(false));
}

#[tokio::test]
async fn dispatch_unknown_plan_is_typed_not_found() {
    let (rt, _mock, _store) = setup().await;
    let err = rt
        .dispatch_decision_tree("brain-plan-none", "anything")
        .await
        .unwrap_err();
    assert!(
        err.downcast_ref::<PlanNotFound>().is_some(),
        "must be the typed marker, got: {err:#}"
    );
}

#[tokio::test]
async fn planner_contract_violations_are_typed_generation_failures() {
    let (rt, mock, _store, caps) = seeded().await;

    // Leaf references an id outside the retrieved candidate set.
    queue_tree(&mock, tree_reply(&caps[0], "brain-fabricated"));
    let err = rt
        .plan_decision_tree("planner-chat", TOPIC_A, 5, 2_000)
        .await
        .unwrap_err();
    assert!(
        err.downcast_ref::<PlanGenerationFailed>().is_some(),
        "unknown capability_id must be a typed failure, got: {err:#}"
    );

    // Completely unparseable reply.
    queue_tree(&mock, "I cannot produce JSON today, sorry.".to_string());
    let err = rt
        .plan_decision_tree("planner-chat", TOPIC_A, 5, 2_000)
        .await
        .unwrap_err();
    assert!(
        err.downcast_ref::<PlanGenerationFailed>().is_some(),
        "unparseable reply must be a typed failure, got: {err:#}"
    );
}

#[tokio::test]
async fn empty_library_cannot_plan() {
    let (rt, mock, _store) = setup().await;
    queue_tree(&mock, "{}".to_string()); // must never be consulted
    let err = rt
        .plan_decision_tree("planner-chat", "anything", 5, 2_000)
        .await
        .unwrap_err();
    assert!(err.downcast_ref::<PlanGenerationFailed>().is_some());
    assert_eq!(mock.call_count(), 0, "no LLM call without candidates");
}

#[tokio::test]
async fn dispatch_or_plan_caches_by_situation_digest() {
    let (rt, mock, _store, caps) = seeded().await;
    queue_tree(&mock, tree_reply(&caps[0], &caps[1]));

    let first = rt
        .dispatch_or_plan("planner-chat", TOPIC_A, 5, false, 2_000)
        .await
        .unwrap();
    assert!(first.planned_fresh);
    assert_eq!(first.outcome.capability_id, caps[0]);
    let chat_calls_after_plan = mock.call_count();

    // Second call with the SAME situation: no queued script, so it must
    // reuse the cached plan without touching the LLM.
    let second = rt
        .dispatch_or_plan("planner-chat", TOPIC_A, 5, false, 2_100)
        .await
        .unwrap();
    assert!(!second.planned_fresh);
    assert_eq!(second.record.id, first.record.id);
    assert_eq!(second.outcome.capability_id, caps[0]);
    assert_eq!(mock.call_count(), chat_calls_after_plan);

    // replan=true mints a fresh plan through the LLM again.
    queue_tree(&mock, tree_reply(&caps[0], &caps[1]));
    let third = rt
        .dispatch_or_plan("planner-chat", TOPIC_A, 5, true, 2_200)
        .await
        .unwrap();
    assert!(third.planned_fresh);
    assert_ne!(third.record.id, first.record.id);
}

#[tokio::test]
async fn corrupt_stored_tree_is_a_plain_500_class_error() {
    let (rt, _mock, store) = setup().await;
    store
        .save_brain_plan(&opencoder_store::BrainPlanRecord {
            id: "brain-plan-bad".into(),
            situation: "s".into(),
            situation_digest: "d".into(),
            chat_model: "m".into(),
            tree_json: "{not json".into(),
            created_at: 1,
        })
        .await
        .unwrap();
    let err = rt
        .dispatch_decision_tree("brain-plan-bad", "s")
        .await
        .unwrap_err();
    assert!(err.downcast_ref::<PlanNotFound>().is_none());
    assert!(err.downcast_ref::<PlanGenerationFailed>().is_none());
}

// ─── pure domain: validate / attach / dispatch, no I/O ─────────────────

fn leaf(id: &str, cap: &str) -> PlanNode {
    PlanNode::Leaf {
        id: id.into(),
        capability_id: cap.into(),
        reason: None,
    }
}

fn two_leaf_tree() -> plan::DecisionTree {
    plan::DecisionTree {
        threshold: 0.5,
        root: PlanNode::Branch {
            id: "b1".into(),
            topic: TOPIC_A.into(),
            reason: None,
            topic_vec: Some(vec![1.0, 0.0]),
            yes: Box::new(leaf("l1", "cap-a")),
            no: Box::new(leaf("l2", "cap-b")),
        },
    }
}

#[test]
fn validate_rejects_structural_violations() {
    let ids = ["cap-a", "cap-b"]
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    assert!(plan::validate(&two_leaf_tree(), &ids).is_ok());
    // Leaf outside the candidate set.
    let mut t = two_leaf_tree();
    if let PlanNode::Branch { yes, .. } = &mut t.root {
        **yes = leaf("l1", "cap-ghost");
    }
    assert!(plan::validate(&t, &ids).is_err());
    // Out-of-range threshold.
    let mut t = two_leaf_tree();
    t.threshold = 1.5;
    assert!(plan::validate(&t, &ids).is_err());
    // Duplicate node id across branches.
    let mut t = two_leaf_tree();
    if let PlanNode::Branch { no, .. } = &mut t.root {
        **no = leaf("l1", "cap-a");
    }
    assert!(plan::validate(&t, &ids).is_err());
}

#[test]
fn attach_topic_vectors_enforces_cardinality_and_dims() {
    let mut t = two_leaf_tree();
    assert!(
        plan::attach_topic_vectors(&mut t.root, &[]).is_err(),
        "0 vecs for 1 topic"
    );
    assert!(
        plan::attach_topic_vectors(&mut t.root, &[vec![1.0], vec![1.0]]).is_err(),
        "2 vecs"
    );
    assert!(
        plan::attach_topic_vectors(&mut t.root, &[vec![]]).is_err(),
        "empty vec"
    );
}

#[test]
fn dispatch_requires_topic_vectors_and_matches_by_cosine() {
    // Aligned vectors: cosine 1.0 → yes.
    let out: DispatchOutcome = plan::dispatch(&two_leaf_tree(), &[3.0, 0.0]).unwrap();
    assert_eq!(out.capability_id, "cap-a");
    // Orthogonal: cosine 0 < 0.5 → no.
    let out = plan::dispatch(&two_leaf_tree(), &[0.0, 2.0]).unwrap();
    assert_eq!(out.capability_id, "cap-b");
    // Missing topic vector: the tree was never fully planned.
    let mut t = two_leaf_tree();
    if let PlanNode::Branch { topic_vec, .. } = &mut t.root {
        *topic_vec = None;
    }
    assert!(plan::dispatch(&t, &[1.0, 0.0]).is_err());
    // Dimension mismatch is an error, not a silent route.
    assert!(plan::dispatch(&two_leaf_tree(), &[1.0, 0.0, 0.0]).is_err());
}

#[test]
fn situation_digest_is_stable_and_trims() {
    let a = opencoder_brain::situation_digest("  same  ");
    assert_eq!(a, opencoder_brain::situation_digest("same"));
    assert_ne!(a, opencoder_brain::situation_digest("different"));
    assert_eq!(a.len(), 32);
}

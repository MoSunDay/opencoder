//! Layered (schema_version 4) control surface: admission gates, the frozen
//! layered request/capability scope, and the read/command routes the workbench
//! codes against. The node owns the projection, so every read is scripted.
use crate::support::Harness;
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use reqwest::Method;
use serde_json::{json, Value};

mod commands;
mod plans;
mod surface;

pub(super) const RUN: &str = "brain-layered-e2e";

/// A two-layer canvas: `scan` feeds `apply`.
pub(super) fn plan() -> Value {
    json!({
        "schema_version": 4,
        "title": "layered canvas",
        "objective": "prove the v4 layered surface",
        "nodes": [
            {"node_id":"scan","title":"Scan","capability_id":"builtin-agent-act",
             "retry":{"max_attempts":2}},
            {"node_id":"apply","title":"Apply","capability_id":"builtin-operator",
             "retry":{"max_attempts":3}}
        ],
        "edges": [{"from":"scan","to":"apply"}],
        "max_rounds": 8
    })
}

pub(super) fn request() -> Value {
    json!({"id":RUN,"schema_version":4,"plan":plan(),"inputs":{}})
}

/// The v4 run is only admitted on a node that advertises the protocol.
pub(super) fn advertise_v4(h: &Harness) {
    h.node
        .set_capability_reply(opencoder_core::fleet::RpcReply::ok(json!({
            "compatible": true,
            "features": ["dag_dynamic_v1", "brain_scheduler_v3", "brain_scheduler_v4"]
        })));
}

/// The layered projection the owning node would own; `phase`/`layer`/
/// `generation` are the only varying parts in these tests.
pub(super) fn snapshot(phase: &str, layer: u32, generation: u64) -> Value {
    json!({"schema_version":4,
        "run":{"run_id":RUN,"phase":phase,"layer":layer,"generation":generation,
            "last_event_seq":0,"error":null,"created_at":1,"updated_at":2},
        "operations":[]})
}

/// Admission of one layered root; returns the receipt body.
pub(super) async fn create(h: &Harness) -> Value {
    let (status, body) = h
        .req(Method::POST, "/api/brain/runs", Some(request()))
        .await;
    assert_eq!(status, 202, "{body}");
    body
}

/// A v3 run created in an earlier server lifetime: frozen request plus index.
/// The prepared assignment writes the index row itself, so this never seeds a
/// conflicting `created_at` for the same execution id.
pub(super) async fn seed_v3_run(h: &Harness, id: &str) {
    let request = opencoder_core::fleet::CreateExecution {
        id: id.into(),
        kind: ExecutionKind::Brain,
        target: None,
        input: json!({"schema_version":3,"scheduler_request":{"schema_version":3}}),
        node_id: None,
    };
    let assignment = opencoder_core::fleet::Assignment {
        runtime: None,
        codex: None,
        definition: None,
        index: opencoder_core::fleet::ExecutionIndex {
            id: id.into(),
            kind: ExecutionKind::Brain,
            node_id: h.node.id.clone(),
            status: ExecutionStatus::Idle,
            created_at: 1,
        },
        request,
    };
    let fingerprint =
        opencoder_core::token_hash(&serde_json::to_string(&assignment.request).unwrap());
    assert!(h
        .state
        .fleet
        .claim_request("execution", id, &fingerprint)
        .await
        .unwrap());
    h.state
        .fleet
        .prepare_assignment(&assignment, &fingerprint)
        .await
        .unwrap();
}

#[tokio::test]
async fn layered_admission_requires_the_v4_advertisement_and_freezes_the_scope() {
    let h = Harness::with_brain_kind().await;
    let (status, body) = h
        .req(Method::POST, "/api/brain/runs", Some(request()))
        .await;
    // A node that cannot negotiate the layered protocol is a placement
    // failure (exactly as in v3), and the reply names the missing feature.
    assert_eq!(status, 503, "{body}");
    assert!(
        body.to_string().contains("brain_scheduler_v4"),
        "a v3-only node must not accept a v4 run: {body}"
    );
    assert!(h.state.fleet.index(RUN).await.unwrap().is_none());

    advertise_v4(&h);
    let receipt = create(&h).await;
    assert_eq!(receipt["schema_version"], json!(4));
    assert_eq!(receipt["run_id"], json!(RUN));
    let assignment = h.state.fleet.assignment(RUN).await.unwrap().unwrap();
    let input = &assignment.request.input;
    assert_eq!(input["schema_version"], json!(4));
    assert_eq!(
        input["layered_request"]["plan"]["title"],
        json!("layered canvas")
    );
    let scope = input["capability_scope"].as_array().unwrap();
    for capability in ["builtin-agent-act", "builtin-operator"] {
        assert!(
            scope.iter().any(|c| c["capability_id"] == capability),
            "frozen scope must name {capability}: {scope:?}"
        );
    }

    // A retry of the identical intent replays the receipt instead of creating
    // a second run.
    let replayed = create(&h).await;
    assert_eq!(replayed, receipt);
    assert_eq!(h.node.journal_ids(), vec![RUN.to_string()]);
}

#[tokio::test]
async fn unknown_or_legacy_schema_versions_are_explicit_errors() {
    let h = Harness::with_brain_kind().await;
    for version in [1, 2, 3, 5] {
        let mut body = request();
        body["schema_version"] = json!(version);
        let (status, reply) = h.req(Method::POST, "/api/brain/runs", Some(body)).await;
        assert_eq!(status, 409, "schema_version {version}: {reply}");
        assert!(
            reply.to_string().contains("migration required"),
            "schema_version {version} must name the migration: {reply}"
        );
    }
    assert!(h.state.fleet.index(RUN).await.unwrap().is_none());
    assert_eq!(h.mock_llm.call_count(), 0);
}

#[tokio::test]
async fn layered_create_requires_a_well_formed_request() {
    let h = Harness::with_brain_kind().await;
    advertise_v4(&h);
    // Cycles are rejected at admission, not on the owning node.
    let mut cyclic = request();
    cyclic["plan"]["edges"] = json!([{"from":"scan","to":"apply"},{"from":"apply","to":"scan"}]);
    let (status, body) = h.req(Method::POST, "/api/brain/runs", Some(cyclic)).await;
    assert_eq!(status, 400, "{body}");
    assert!(body.to_string().contains("cycle"), "{body}");
    // An unknown capability never reaches placement either.
    let mut unknown = request();
    unknown["plan"]["nodes"][0]["capability_id"] = json!("not-registered");
    let (status, body) = h.req(Method::POST, "/api/brain/runs", Some(unknown)).await;
    assert_eq!(status, 400, "{body}");
    assert!(body.to_string().contains("not-registered"), "{body}");
    assert!(h.node.journal_request(RUN).is_none());
    assert!(h.state.fleet.assignment(RUN).await.unwrap().is_none());
}

#[tokio::test]
async fn layered_nesting_is_bounded_and_requires_its_parent() {
    let h = Harness::with_brain_kind().await;
    advertise_v4(&h);
    let nested = "brain-layered-nested";
    let parent =
        json!({"run_id":RUN,"operation_id":format!("{RUN}#l1#scan#a1"),"node_id":"scan","layer":1});
    for (depth, binding, expected) in [
        (4, Some(parent.clone()), "nesting depth exceeded"),
        (1, None, "parent must agree"),
    ] {
        let mut body = request();
        body["id"] = json!(nested);
        body["depth"] = json!(depth);
        if let Some(binding) = binding {
            body["parent"] = binding;
        }
        let (status, reply) = h.req(Method::POST, "/api/brain/runs", Some(body)).await;
        assert_eq!(status, 400, "depth {depth}: {reply}");
        assert!(
            reply.to_string().contains(expected),
            "depth {depth}: {reply}"
        );
    }
    assert!(h.node.journal_request(nested).is_none());

    // A forged parent cannot create an unrelated nested run.
    let mut body = request();
    body["id"] = json!(nested);
    body["depth"] = json!(1);
    body["parent"] = parent;
    let (status, receipt) = h.req(Method::POST, "/api/brain/runs", Some(body)).await;
    assert_eq!(status, 400, "{receipt}");
    assert!(h.state.fleet.assignment(nested).await.unwrap().is_none());
}

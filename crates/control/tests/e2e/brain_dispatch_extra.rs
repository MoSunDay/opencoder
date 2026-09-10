//! Extra brain dispatch semantics: replan over the digest cache, request
//! overrides, keyed conflict classes (changed input, candidate kind
//! mismatch), unconfirmed-node receipts, custom execution ids, frozen
//! admission and non-agent (team) targets.

use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use reqwest::Method;
use serde_json::{json, Value};

use crate::support::Harness;

const SITUATION: &str = "resize the fleet tonight";
const THRESH: f64 = 0.98;

fn capability_payload(summary: &str) -> Value {
    json!({
        "capability_type": "tool-usage",
        "summary": summary,
        "input_desc": "a work request",
        "output_desc": "completed work",
        "eng_inputs": ["exemplar input"],
    })
}

async fn seed_cap(h: &Harness, summary: &str) -> String {
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/capabilities",
            Some(capability_payload(summary)),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    body["capability"]["id"].as_str().unwrap().to_string()
}

/// Queues one planner round-trip whose branch topic is exactly SITUATION so
/// the walk takes the yes leaf routed to `cap`.
fn queue_plan(h: &Harness, cap: &str) {
    let reply = format!(
        "{{\"threshold\":{THRESH},\"root\":{{\"id\":\"b1\",\"kind\":\"branch\",\"topic\":\"{SITUATION}\",\"yes\":{{\"id\":\"l1\",\"kind\":\"leaf\",\"capability_id\":\"{cap}\",\"reason\":\"routed\"}},\"no\":{{\"id\":\"l2\",\"kind\":\"leaf\",\"capability_id\":\"{cap}\",\"reason\":\"routed\"}}}}}}"
    );
    h.mock_llm.queue_script(vec![
        opencoder_llm::LlmEvent::TextDelta(reply.clone()),
        opencoder_llm::LlmEvent::Completed {
            text: reply,
            tool_calls: Vec::new(),
            usage: None,
        },
    ]);
}

async fn bind_agent(h: &Harness, cap: &str) {
    let (status, body) = h
        .req(
            Method::PUT,
            &format!("/api/brain/capabilities/{cap}/target"),
            Some(json!({"kind": "agent", "target": "act"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
}

/// `replan: true` must bypass the situation-digest cache: the second
/// dispatch consumes the second planner script (a different capability)
/// even though a cached plan for the same situation exists, while the third
/// call without replan reuses that newest plan without touching the LLM.
#[tokio::test]
async fn dispatch_replan_forces_fresh_plan_over_digest_cache() {
    let h = Harness::new().await;
    let cap_a = seed_cap(&h, "plan variant a").await;
    let cap_b = seed_cap(&h, "plan variant b").await;
    bind_agent(&h, &cap_a).await;
    bind_agent(&h, &cap_b).await;

    queue_plan(&h, &cap_a);
    let (status, first) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({"situation": SITUATION})),
        )
        .await;
    assert_eq!(status, 202, "{first}");
    assert_eq!(first["capability_id"], json!(cap_a));
    assert_eq!(first["planned_fresh"], json!(true));

    queue_plan(&h, &cap_b);
    let (status, second) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({"situation": SITUATION, "replan": true})),
        )
        .await;
    assert_eq!(status, 202, "{second}");
    assert_eq!(
        second["capability_id"],
        json!(cap_b),
        "replan used the new LLM output"
    );
    assert_eq!(second["planned_fresh"], json!(true));
    assert_ne!(second["plan_id"], first["plan_id"]);

    // Without replan the newest plan for the digest is reused verbatim.
    let (status, third) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({"situation": SITUATION})),
        )
        .await;
    assert_eq!(status, 202, "{third}");
    assert_eq!(third["planned_fresh"], json!(false));
    assert_eq!(third["capability_id"], json!(cap_b));
    assert_eq!(third["plan_id"], second["plan_id"]);
}

/// `top_k` and `model` overrides are accepted on dispatch; the model
/// override is pinned through the persisted plan's provenance field.
#[tokio::test]
async fn dispatch_accepts_top_k_and_model_overrides() {
    let h = Harness::new().await;
    let cap = seed_cap(&h, "override probes").await;
    bind_agent(&h, &cap).await;
    queue_plan(&h, &cap);

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({
                "situation": SITUATION,
                "top_k": 3,
                "model": "planner-override-model",
            })),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    let plan_id = body["plan_id"].as_str().unwrap().to_string();

    let (status, plan) = h
        .req(Method::GET, &format!("/api/brain/plans/{plan_id}"), None)
        .await;
    assert_eq!(status, 200, "{plan}");
    assert_eq!(plan["plan"]["chat_model"], json!("planner-override-model"));
}

/// Keyed dispatch: a request_id reused with different input is a 409 even
/// though the node still holds the original receipt; an execution index
/// found under the minted id but with a different kind is a 409 candidate
/// conflict before any planning happens.
#[tokio::test]
async fn keyed_conflicts_on_changed_input_and_candidate_kind() {
    let h = Harness::new().await;
    let cap = seed_cap(&h, "conflict capability").await;
    bind_agent(&h, &cap).await;
    queue_plan(&h, &cap);

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({"situation": SITUATION, "request_id": "r2"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["execution"]["id"], json!("agent-r2"));

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({"situation": "a different situation", "request_id": "r2"})),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(
        body["error"],
        json!("request_id already used with different input")
    );

    // A pre-existing index row for the minted id but of another kind.
    h.put_index("agent-r1", ExecutionKind::Team, ExecutionStatus::Idle)
        .await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({"situation": SITUATION, "request_id": "r1"})),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(
        body["error"],
        json!("request_id already used with different input")
    );
}

/// A keyed replay whose index exists only in the control DB (no node
/// journal) is a 503: the owning node has not confirmed the request, which
/// the scripted node answers with a 404.
#[tokio::test]
async fn keyed_unconfirmed_node_receipt_is_503() {
    let h = Harness::new().await;
    h.put_index("agent-r9", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({"situation": SITUATION, "request_id": "r9"})),
        )
        .await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(
        body["error"],
        json!("owning node has not confirmed this brain request")
    );
}

/// Unkeyed dispatch honors a client-supplied execution id (still kind
/// prefixed), and once the server is drained the unkeyed path is refused
/// with 503 before any planner work.
#[tokio::test]
async fn unkeyed_custom_id_and_frozen_admission() {
    let h = Harness::new().await;
    let cap = seed_cap(&h, "custom id capability").await;
    bind_agent(&h, &cap).await;
    queue_plan(&h, &cap);

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({"situation": SITUATION, "id": "agent-custom-1"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["execution"]["id"], json!("agent-custom-1"));

    let (status, _) = h.req(Method::POST, "/api/admin/drain", None).await;
    assert_eq!(status, 200);
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({"situation": SITUATION})),
        )
        .await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"], json!("server admission is frozen"));
}

/// A capability bound to a team target dispatches a team execution: the
/// minted id uses the team prefix and the receipt kind is team.
#[tokio::test]
async fn team_target_dispatches_team_execution() {
    let h = Harness::new().await;
    let team = json!({
        "name": "relay-team",
        "captain": "act",
        "members": [{"agent": "act"}],
    });
    let (status, body) = h.req(Method::POST, "/api/teams", Some(team)).await;
    assert_eq!(status, 200, "{body}");

    let cap = seed_cap(&h, "team routed capability").await;
    let (status, body) = h
        .req(
            Method::PUT,
            &format!("/api/brain/capabilities/{cap}/target"),
            Some(json!({"kind": "team", "target": "relay-team"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    queue_plan(&h, &cap);
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({"situation": SITUATION, "request_id": "r4"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["execution"]["kind"], json!("team"));
    assert_eq!(body["execution"]["id"], json!("team-r4"));
    assert_eq!(body["execution"]["node_id"], json!("node-e2e"));
}

//! Brain surface: capability CRUD + search (mock embedder), target binding,
//! plan lifecycle with a scripted planner LLM, preview and dispatch
//! (unkeyed execution + keyed idempotent receipts).

use reqwest::Method;
use serde_json::json;

use crate::support::{Harness, TOKEN};

const TOPIC_A: &str = "db migration plan";
const THRESH: f64 = 0.98;

fn capability_payload(summary: &str) -> serde_json::Value {
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

/// Queues one planner LLM round-trip returning a decision tree that
/// routes TOPIC_A to `cap`.
fn queue_plan(h: &Harness, cap: &str) {
    let reply = format!(
        "{{\"threshold\":{THRESH},\"root\":{{\"id\":\"b1\",\"kind\":\"branch\",\"topic\":\"{TOPIC_A}\",\"yes\":{{\"id\":\"l1\",\"kind\":\"leaf\",\"capability_id\":\"{cap}\",\"reason\":\"db work\"}},\"no\":{{\"id\":\"l2\",\"kind\":\"leaf\",\"capability_id\":\"{cap}\",\"reason\":\"db work\"}}}}}}"
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

#[tokio::test]
async fn capability_crud_search_and_target_binding() {
    let h = Harness::new().await;
    let id = seed_cap(&h, TOPIC_A).await;

    let (status, body) = h.req(Method::GET, "/api/brain/capabilities", None).await;
    assert_eq!(status, 200, "{body}");
    assert!(body["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["capability"]["id"] == json!(id)));

    let (status, body) = h
        .req(Method::GET, &format!("/api/brain/capabilities/{id}"), None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["capability"]["summary"], json!(TOPIC_A));
    let (status, body) = h
        .req(Method::GET, "/api/brain/capabilities/cap-none", None)
        .await;
    assert_eq!(status, 404, "{body}");

    let updated = capability_payload("updated summary");
    let (status, body) = h
        .req(
            Method::PUT,
            &format!("/api/brain/capabilities/{id}"),
            Some(updated),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["capability"]["summary"], json!("updated summary"));

    // Exact-text search hits the (re-embedded) capability.
    let typed: opencoder_brain::CapabilityInput =
        serde_json::from_value(capability_payload("updated summary")).unwrap();
    let composed = opencoder_brain::domain::compose_embed_text(&typed);
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/search",
            Some(json!({"query": composed, "k": 5})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body["hits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hit| hit["capability"]["id"] == json!(id)),
        "{body}"
    );
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/search",
            Some(json!({"query": "  "})),
        )
        .await;
    assert_eq!(status, 400, "{body}");

    // Unbound target is null; binding persists and validates.
    let (status, body) = h
        .req(
            Method::GET,
            &format!("/api/brain/capabilities/{id}/target"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["target"], json!(null));
    let (status, body) = h
        .req(
            Method::PUT,
            &format!("/api/brain/capabilities/{id}/target"),
            Some(json!({"kind": "agent", "target": "act"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["kind"], json!("agent"));
    let (status, body) = h
        .req(
            Method::PUT,
            &format!("/api/brain/capabilities/{id}/target"),
            Some(json!({"kind": "bogus", "target": "x"})),
        )
        .await;
    // Unknown kind fails CapabilityTarget deserialization → axum 422.
    assert_eq!(status, 422, "{body}");
    let (status, body) = h
        .req(
            Method::PUT,
            "/api/brain/capabilities/cap-none/target",
            Some(json!({"kind": "agent", "target": "act"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");

    let (status, body) = h
        .req(
            Method::DELETE,
            &format!("/api/brain/capabilities/{id}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["deleted"], json!(id));
}

#[tokio::test]
async fn plan_lifecycle_with_scripted_planner() {
    let h = Harness::new().await;
    let cap_a = seed_cap(&h, TOPIC_A).await;
    let cap_b = seed_cap(&h, "write unit tests").await;
    let reply = format!(
        "{{\"threshold\":{THRESH},\"root\":{{\"id\":\"b1\",\"kind\":\"branch\",\"topic\":\"{TOPIC_A}\",\"yes\":{{\"id\":\"l1\",\"kind\":\"leaf\",\"capability_id\":\"{cap_a}\",\"reason\":\"db work\"}},\"no\":{{\"id\":\"l2\",\"kind\":\"leaf\",\"capability_id\":\"{cap_b}\",\"reason\":\"test work\"}}}}}}"
    );
    h.mock_llm.queue_script(vec![
        opencoder_llm::LlmEvent::TextDelta(reply.clone()),
        opencoder_llm::LlmEvent::Completed {
            text: reply,
            tool_calls: Vec::new(),
            usage: None,
        },
    ]);

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/plans",
            Some(json!({"situation": TOPIC_A})),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let plan_id = body["plan"]["id"].as_str().unwrap().to_string();
    assert!(plan_id.starts_with("brain-plan-"), "{body}");
    assert_eq!(body["tree"]["root"]["kind"], json!("branch"));

    let (status, body) = h
        .req(Method::GET, &format!("/api/brain/plans/{plan_id}"), None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["plan"]["id"], json!(plan_id));
    let (status, body) = h
        .req(Method::GET, "/api/brain/plans/brain-plan-none", None)
        .await;
    assert_eq!(status, 404, "{body}");

    // Preview routes the branch topic to the yes leaf without dispatching.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/preview",
            Some(json!({"situation": TOPIC_A, "plan_id": plan_id})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["capability_id"], json!(cap_a));
    assert!(!body["path"].as_array().unwrap().is_empty(), "{body}");
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/preview",
            Some(json!({"situation": ""})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
}

#[tokio::test]
async fn dispatch_unkeyed_creates_execution_and_keyed_is_idempotent() {
    let h = Harness::new().await;
    let cap = seed_cap(&h, TOPIC_A).await;
    let (status, _) = h
        .req(
            Method::PUT,
            &format!("/api/brain/capabilities/{cap}/target"),
            Some(json!({"kind": "agent", "target": "act"})),
        )
        .await;
    assert_eq!(status, 200);

    // Each planner round-trip consumes one scripted LLM reply: the unkeyed
    // dispatch and the keyed first call each plan once; the keyed replay is
    // served from the durable receipt without consulting the LLM.
    queue_plan(&h, &cap);
    queue_plan(&h, &cap);

    // Unkeyed: full decision + execution in one call.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({"situation": TOPIC_A, "node_id": "node-e2e"})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["capability_id"], json!(cap));
    assert_eq!(body["execution"]["kind"], json!("agent"));
    assert_eq!(body["execution"]["node_id"], json!("node-e2e"));

    // Keyed first call mints agent-<request_id>; replay returns the same
    // execution through the node's durable receipt.
    let (status, first) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({"situation": TOPIC_A, "request_id": "r1"})),
        )
        .await;
    assert_eq!(status, 202, "{first}");
    assert_eq!(first["execution"]["id"], json!("agent-r1"));
    let (status, replay) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({"situation": TOPIC_A, "request_id": "r1"})),
        )
        .await;
    assert_eq!(status, 202, "{replay}");
    assert_eq!(replay["execution"]["id"], json!("agent-r1"));
    assert_eq!(
        replay["execution"]["created_at"],
        first["execution"]["created_at"]
    );

    // Validation: empty situation / bad request_id.
    for bad in [
        json!({"situation": ""}),
        json!({"situation": "x", "request_id": "bad id!"}),
        json!({"situation": "x", "id": "agent-q", "request_id": "q2"}),
    ] {
        let (status, body) = h.req(Method::POST, "/api/brain/dispatch", Some(bad)).await;
        assert_eq!(status, 400, "{body}");
    }
}

#[tokio::test]
async fn dispatch_without_bindable_target_is_rejected() {
    let h = Harness::new().await;
    let cap = seed_cap(&h, "unbound capability").await;
    // The planner routes successfully first; only then does the missing
    // target binding surface as a 400 (no LLM script would yield a 502).
    queue_plan(&h, &cap);
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/dispatch",
            Some(json!({"situation": "unbound capability"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["error"],
        json!(format!("capability {cap} has no executable target"))
    );
}

// Token sanity: brain endpoints are behind the bearer like everything else.
#[tokio::test]
async fn brain_requires_bearer() {
    let h = Harness::new().await;
    let resp = h
        .req_raw(
            Method::GET,
            "/api/brain/capabilities",
            None,
            Some(&format!("not-{TOKEN}")),
        )
        .await;
    assert_eq!(resp.status(), 401);
}

// ─── extra coverage: validation edges, search k policy, target guard ───

/// Field-level validation rejects blank summaries and oversized exemplar
/// inputs (both the per-capacity entry count and the per-entry length);
/// unknown ids are a 404 for update and delete alike.
#[tokio::test]
async fn capability_validation_edges_and_unknown_ids() {
    let h = Harness::new().await;

    let mut blank = capability_payload("   ");
    blank["summary"] = json!("   ");
    let (status, body) = h
        .req(Method::POST, "/api/brain/capabilities", Some(blank))
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], json!("summary must not be empty"));

    // eng_inputs capacity is 64 entries; 65 is refused before embedding.
    let mut too_many = capability_payload("too many exemplars");
    too_many["eng_inputs"] = json!(vec!["exemplar"; 65]);
    let (status, body) = h
        .req(Method::POST, "/api/brain/capabilities", Some(too_many))
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], json!("eng_inputs exceeds 64 entries"));

    // One entry above the 4000-char per-entry cap is refused too.
    let mut too_long = capability_payload("one huge exemplar");
    too_long["eng_inputs"] = json!(["x".repeat(4001)]);
    let (status, body) = h
        .req(Method::POST, "/api/brain/capabilities", Some(too_long))
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], json!("eng_inputs[0] exceeds 4000 chars"));

    let valid = capability_payload("valid for update probes");
    let (status, body) = h
        .req(
            Method::PUT,
            "/api/brain/capabilities/cap-unknown",
            Some(valid.clone()),
        )
        .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(
        body["error"],
        json!("brain capability not found: cap-unknown")
    );
    let (status, body) = h
        .req(Method::DELETE, "/api/brain/capabilities/cap-unknown", None)
        .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(
        body["error"],
        json!("brain capability not found: cap-unknown")
    );
}

/// `k` policy on search: omitted → default 10, absurd values clamp to the
/// hard ceiling of 50 (the store LIMIT), and every hit carries the
/// capability record plus its vector distance.
#[tokio::test]
async fn search_k_default_and_clamp() {
    let h = Harness::new().await;
    for i in 0..55 {
        seed_cap(&h, &format!("bulk capability {i}")).await;
    }

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/search",
            Some(json!({"query": "bulk capability"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let hits = body["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 10, "default k is 10: {body}");
    assert!(
        hits.iter()
            .all(|hit| hit["capability"]["id"].as_str().is_some()
                && hit["distance"].as_f64().is_some()),
        "{body}"
    );

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/search",
            Some(json!({"query": "bulk capability", "k": 999})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["hits"].as_array().unwrap().len(),
        50,
        "k clamps to 50 even with 55 seeded: {body}"
    );
}

/// Target binding guard: serde-valid kinds outside the routable set
/// (project) and blank target names are 400s; rebinding overwrites (the
/// definition row keeps exactly one newest target); an unknown capability
/// id still answers 200 with a null target — the read pins the
/// `{"target": null}` contract instead of a 404.
#[tokio::test]
async fn target_guard_rebind_and_unknown_id_is_null() {
    let h = Harness::new().await;
    let id = seed_cap(&h, "guard target capability").await;
    let path = format!("/api/brain/capabilities/{id}/target");

    let (status, body) = h
        .req(
            Method::PUT,
            &path,
            Some(json!({"kind": "project", "target": "x"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["error"],
        json!("capability target must name an agent, team or workflow")
    );
    let (status, body) = h
        .req(
            Method::PUT,
            &path,
            Some(json!({"kind": "agent", "target": "   "})),
        )
        .await;
    assert_eq!(status, 400, "{body}");

    let (status, body) = h
        .req(
            Method::PUT,
            &path,
            Some(json!({"kind": "agent", "target": "act"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["kind"], json!("agent"));
    // Rebinding replaces: the read shows only the newest binding.
    let (status, body) = h
        .req(
            Method::PUT,
            &path,
            Some(json!({"kind": "team", "target": "crew"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["kind"], json!("team"));
    let (status, body) = h.req(Method::GET, &path, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["target"], json!({"kind": "team", "target": "crew"}));

    let (status, body) = h
        .req(Method::GET, "/api/brain/capabilities/cap-none/target", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["target"], json!(null), "unknown id is null, not 404");
}

/// Plans admission gate + planner fault surface + dynamic preview: blank
/// situations are 400 before any LLM work, an unparseable planner reply is
/// a 502, an unknown plan_id in preview is 404, a preview without plan_id
/// plans fresh (planned_fresh) through the scripted planner, and a drained
/// server refuses new plans with 503.
#[tokio::test]
async fn plans_gates_planner_faults_and_dynamic_preview() {
    let h = Harness::new().await;
    let cap = seed_cap(&h, TOPIC_A).await;

    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/plans",
            Some(json!({"situation": "  "})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], json!("situation must not be empty"));

    // Garbage planner text (valid script, non-JSON payload) → 502.
    h.mock_llm.queue_script(vec![
        opencoder_llm::LlmEvent::TextDelta("definitely not a decision tree".into()),
        opencoder_llm::LlmEvent::Completed {
            text: "definitely not a decision tree".into(),
            tool_calls: Vec::new(),
            usage: None,
        },
    ]);
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/plans",
            Some(json!({"situation": TOPIC_A})),
        )
        .await;
    assert_eq!(status, 502, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("planner reply unparseable"),
        "{body}"
    );

    // Unknown plan in preview: 404 without consulting the LLM.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/preview",
            Some(json!({"situation": TOPIC_A, "plan_id": "brain-plan-none"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");

    // Dynamic mode (no plan_id): plans fresh through the scripted planner.
    queue_plan(&h, &cap);
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/preview",
            Some(json!({"situation": TOPIC_A})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["capability_id"], json!(cap));
    assert_eq!(body["planned_fresh"], json!(true), "{body}");

    // Drained server: the plans admission gate refuses with 503.
    let (status, _) = h.req(Method::POST, "/api/admin/drain", None).await;
    assert_eq!(status, 200);
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/plans",
            Some(json!({"situation": "post drain"})),
        )
        .await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"], json!("server admission is frozen"));
}

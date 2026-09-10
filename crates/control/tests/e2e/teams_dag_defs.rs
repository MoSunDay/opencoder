//! Control-plane-owned definitions: team CRUD with validation and DAG
//! definition CRUD (save, list, fetch, delete).

use reqwest::Method;
use serde_json::json;

use crate::support::Harness;

const TEAM: &str = r#"{"name":"demo","captain":"act","members":[
    {"agent":"act"},
    {"agent":"plan"}]}"#;

const SPEC: &str = r#"{"name":"etl-demo","steps":[
    {"name":"fetch","kind":{"type":"wasm","command":"tool.wasm"}},
    {"name":"load","depends_on":["fetch"],"kind":{"type":"wasm","command":"tool.wasm"}}]}"#;

#[tokio::test]
async fn teams_roundtrip_and_validation() {
    let h = Harness::new().await;
    let (status, body) = h.req(Method::GET, "/api/teams", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["teams"], json!([]));

    let team: serde_json::Value = serde_json::from_str(TEAM).unwrap();
    let (status, body) = h.req(Method::POST, "/api/teams", Some(team.clone())).await;
    assert_eq!(status, 200, "{body}");
    // The minimal `{"agent"}` wire shape round-trips: capabilities default
    // to an empty list (they are control-plane-frozen, never user input).
    assert_eq!(body["name"], team["name"]);
    assert_eq!(
        body["members"],
        json!([
            {"agent":"act","capabilities":[]},
            {"agent":"plan","capabilities":[]},
        ])
    );

    let (status, body) = h.req(Method::GET, "/api/teams", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["teams"].as_array().unwrap().len(), 1);
    assert_eq!(body["teams"][0]["captain"], json!("act"));

    // Validation: reserved name, foreign captain, empty members, dup agents.
    for bad in [
        json!({"name":"system","captain":"act","members":[{"agent":"a"}]}),
        json!({"name":"demo2","captain":"mx","members":[{"agent":"a"}]}),
        json!({"name":"demo3","captain":"act","members":[]}),
        json!({"name":"demo4","captain":"a","members":[{"agent":"a"},{"agent":"a"}]}),
    ] {
        let (status, body) = h.req(Method::POST, "/api/teams", Some(bad)).await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(
            body["error"],
            json!("team requires a unique non-empty agent per member and a captain belonging to the team; system is reserved")
        );
    }
}

#[tokio::test]
async fn dag_definitions_crud() {
    let h = Harness::new().await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/dag/defs",
            Some(json!({"spec": serde_json::from_str::<serde_json::Value>(SPEC).unwrap()})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("etl-demo"));
    assert_eq!(body["name"], json!("etl-demo"));

    let (status, body) = h.req(Method::GET, "/api/dag/defs", None).await;
    assert_eq!(status, 200, "{body}");
    let defs = body.as_array().unwrap();
    assert_eq!(defs[0]["id"], json!("etl-demo"));
    assert_eq!(defs[0]["spec"]["steps"].as_array().unwrap().len(), 2);

    let (status, body) = h.req(Method::GET, "/api/dag/defs/etl-demo", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["spec"]["name"], json!("etl-demo"));
    let (status, body) = h.req(Method::GET, "/api/dag/defs/none", None).await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], json!("DAG not found"));

    // Specs with dangling dependencies are rejected by domain validation.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/dag/defs",
            Some(json!({"spec": {"name": "bad", "steps": [
            {"name": "a", "depends_on": ["ghost"], "kind": {"type":"wasm","command":"tool.wasm"}}]}})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("depends on unknown step"),
        "{body}"
    );

    let (status, body) = h.req(Method::DELETE, "/api/dag/defs/etl-demo", None).await;
    assert_eq!(status, 200, "{body}");
    let (status, _) = h.req(Method::GET, "/api/dag/defs/etl-demo", None).await;
    assert_eq!(status, 404);
}

fn member(agent: &str) -> serde_json::Value {
    json!({"agent": agent})
}

fn team_named(name: &str) -> serde_json::Value {
    json!({"name": name, "captain": "act", "members": [member("act")]})
}

/// Team names are lowercase share slugs: no uppercase, no underscore, <=64
/// chars; members need a non-empty agent.
#[tokio::test]
async fn team_name_charset_and_member_fields_are_validated() {
    let h = Harness::new().await;
    let mut bad: Vec<(String, serde_json::Value)> = vec![
        ("uppercase".into(), team_named("TeamA")),
        ("underscore".into(), team_named("a_b")),
        ("too long".into(), team_named(&"a".repeat(65))),
        (
            "blank agent".into(),
            json!({"name": "ok-a", "captain": "act", "members": [member("  ")]}),
        ),
    ];
    for (label, body) in bad.drain(..) {
        let (status, reply) = h.req(Method::POST, "/api/teams", Some(body)).await;
        assert_eq!(status, 400, "{label}: {reply}");
        assert!(
            reply["error"]
                .as_str()
                .unwrap()
                .contains("unique non-empty agent per member"),
            "{label}: {reply}"
        );
    }
    // The adjacent legal name (dash + digits) still passes.
    let (status, reply) = h
        .req(Method::POST, "/api/teams", Some(team_named("a-b2")))
        .await;
    assert_eq!(status, 200, "{reply}");
}

/// Upserting an existing name replaces the stored shape (definitions are
/// keyed by name, not append-only).
#[tokio::test]
async fn team_upsert_replaces_the_stored_shape() {
    let h = Harness::new().await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/teams",
            Some(json!({"name": "demo", "captain": "act", "members": [
                member("act"), member("plan")]})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = h
        .req(
            Method::POST,
            "/api/teams",
            Some(json!({"name": "demo", "captain": "explore",
                "members": [member("explore")]})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["captain"], json!("explore"));

    let (status, body) = h.req(Method::GET, "/api/teams", None).await;
    assert_eq!(status, 200, "{body}");
    let teams = body["teams"].as_array().unwrap();
    assert_eq!(teams.len(), 1, "{body}");
    assert_eq!(teams[0]["captain"], json!("explore"));
    assert_eq!(teams[0]["members"].as_array().unwrap().len(), 1);
}

/// A legacy "system" team definition stored directly in the fleet store is
/// filtered from the list (the name is reserved and not dispatchable).
#[tokio::test]
async fn legacy_system_team_definition_is_hidden_from_the_list() {
    let h = Harness::new().await;
    h.state
        .fleet
        .put_definition(
            "team",
            "system",
            &json!({"name": "system", "captain": "a", "members": [member("a")]}),
        )
        .await
        .unwrap();
    let (status, body) = h.req(Method::GET, "/api/teams", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["teams"], json!([]));

    // Regular teams still list alongside the hidden one.
    let (status, body) = h
        .req(Method::POST, "/api/teams", Some(team_named("demo")))
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = h.req(Method::GET, "/api/teams", None).await;
    assert_eq!(status, 200, "{body}");
    let teams = body["teams"].as_array().unwrap();
    assert_eq!(teams.len(), 1, "{body}");
    assert_eq!(teams[0]["name"], json!("demo"));
}

/// Resolve freezes each member agent's brain-bound capability summaries into
/// the pinned definition the node receives: bound agent → summaries,
/// unbound agent → empty list. The stored team keeps the minimal shape.
#[tokio::test]
async fn team_resolve_freezes_member_capability_snapshots() {
    let h = Harness::new().await;
    let team = json!({"name": "pin-team", "captain": "act", "members": [
        member("act"), member("plan")]});
    let (status, body) = h.req(Method::POST, "/api/teams", Some(team)).await;
    assert_eq!(status, 200, "{body}");

    let summary = "db migration snapshots";
    let (status, body) = h
        .req(
            Method::POST,
            "/api/brain/capabilities",
            Some(json!({
                "capability_type": "tool-usage",
                "summary": summary,
                "input_desc": "a work request",
                "output_desc": "completed work",
                "eng_inputs": ["exemplar input"],
            })),
        )
        .await;
    assert_eq!(status, 201, "{body}");
    let cap = body["capability"]["id"].as_str().unwrap().to_string();
    let (status, body) = h
        .req(
            Method::PUT,
            &format!("/api/brain/capabilities/{cap}/target"),
            Some(json!({"kind": "agent", "target": "act"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    let (status, body) = h
        .req(
            Method::POST,
            "/api/executions",
            Some(
                json!({"id": "team-pin-1", "kind": "team", "target": "pin-team",
                "node_id": "node-e2e"}),
            ),
        )
        .await;
    assert_eq!(status, 202, "{body}");

    let pinned = h
        .node
        .pinned_definition("team-pin-1")
        .expect("assignment reached the node");
    let by_agent = |agent: &str| {
        pinned["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["agent"] == json!(agent))
            .unwrap_or_else(|| panic!("member {agent} missing: {pinned}"))
            .clone()
    };
    assert_eq!(by_agent("act")["capabilities"], json!([summary]));
    assert_eq!(by_agent("plan")["capabilities"], json!([]));

    // The stored team itself stays minimal — only the pinned snapshot grows.
    let (status, body) = h.req(Method::GET, "/api/teams", None).await;
    assert_eq!(status, 200, "{body}");
    let stored = body["teams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == json!("pin-team"))
        .unwrap()
        .clone();
    assert_eq!(stored["members"][0]["capabilities"], json!([]));
}

fn wasm_step(name: &str, depends_on: serde_json::Value) -> serde_json::Value {
    json!({"name": name, "depends_on": depends_on, "kind": {"type":"wasm","command":"tool.wasm"}})
}

/// save_dag rejects unparsable and invalid specs with the domain validator's
/// aggregated messages; a bare spec (no `{"spec": ...}` wrapper) is accepted,
/// and deleting an unknown definition is an idempotent success.
#[tokio::test]
async fn dag_definition_spec_validation_table() {
    let h = Harness::new().await;
    let table: Vec<(&str, serde_json::Value, &str)> = vec![
        (
            "steps wrong type",
            json!({"name": "bad", "steps": "nope"}),
            "invalid type",
        ),
        ("missing steps", json!({"name": "bad"}), "missing field"),
        (
            "empty steps",
            json!({"name": "bad", "steps": []}),
            "spec.steps must not be empty",
        ),
        (
            "duplicate step names",
            json!({"name": "bad", "steps": [wasm_step("a", json!([])), wasm_step("a", json!([]))]}),
            "duplicate step name",
        ),
        (
            "dependency cycle",
            json!({"name": "bad", "steps": [
                wasm_step("a", json!(["b"])), wasm_step("b", json!(["a"]))]}),
            "cycle detected",
        ),
    ];
    for (label, spec, expected) in table {
        let (status, body) = h
            .req(Method::POST, "/api/dag/defs", Some(json!({"spec": spec})))
            .await;
        assert_eq!(status, 400, "{label}: {body}");
        assert!(
            body["error"].as_str().is_some_and(|e| e.contains(expected)),
            "{label}: {body}"
        );
    }

    // Bare-spec body: the whole payload is the spec (unwrap_or branch).
    let (status, body) = h
        .req(
            Method::POST,
            "/api/dag/defs",
            Some(json!({"name": "bare-demo", "steps": [wasm_step("only", json!([]))]})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("bare-demo"));
    assert_eq!(body["spec"]["steps"].as_array().unwrap().len(), 1);
    let (status, body) = h.req(Method::GET, "/api/dag/defs/bare-demo", None).await;
    assert_eq!(status, 200, "{body}");

    // Deleting an unknown definition is a no-op success (pinned contract).
    let (status, body) = h.req(Method::DELETE, "/api/dag/defs/none", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({"ok": true}));
}

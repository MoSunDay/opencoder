//! Control-plane-owned definitions: team CRUD with validation and DAG
//! definition CRUD (save, list, fetch, delete).

use reqwest::Method;
use serde_json::json;

use crate::support::Harness;

const TEAM: &str = r#"{"name":"demo","captain":"m1","members":[
    {"id":"m1","agent":"act","role":"captain"},
    {"id":"m2","agent":"plan","role":"advisor"}]}"#;

const SPEC: &str = r#"{"name":"etl-demo","steps":[
    {"name":"fetch","kind":{"type":"python","code":"x=1"}},
    {"name":"load","depends_on":["fetch"],"kind":{"type":"python","code":"y=2"}}]}"#;

#[tokio::test]
async fn teams_roundtrip_and_validation() {
    let h = Harness::new().await;
    let (status, body) = h.req(Method::GET, "/api/teams", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["teams"], json!([]));

    let team: serde_json::Value = serde_json::from_str(TEAM).unwrap();
    let (status, body) = h.req(Method::POST, "/api/teams", Some(team.clone())).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, team);

    let (status, body) = h.req(Method::GET, "/api/teams", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["teams"].as_array().unwrap().len(), 1);
    assert_eq!(body["teams"][0]["captain"], json!("m1"));

    // Validation: reserved name, foreign captain, empty members, dup ids.
    for bad in [
        json!({"name":"system","captain":"m1","members":[{"id":"m1","agent":"a","role":"r"}]}),
        json!({"name":"demo2","captain":"mx","members":[{"id":"m1","agent":"a","role":"r"}]}),
        json!({"name":"demo3","captain":"m1","members":[]}),
        json!({"name":"demo4","captain":"m1","members":[
            {"id":"m1","agent":"a","role":"r"},{"id":"m1","agent":"b","role":"r"}]}),
    ] {
        let (status, body) = h.req(Method::POST, "/api/teams", Some(bad)).await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(
            body["error"],
            json!("team requires a unique member id, agent and role for each member, and a captain belonging to the team; system is reserved")
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
            {"name": "a", "depends_on": ["ghost"], "kind": {"type": "python", "code": "x=1"}}]}})),
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

fn member(id: &str, agent: &str, role: &str) -> serde_json::Value {
    json!({"id": id, "agent": agent, "role": role})
}

fn team_named(name: &str) -> serde_json::Value {
    json!({"name": name, "captain": "m1", "members": [member("m1", "act", "captain")]})
}

/// Team names are lowercase share slugs: no uppercase, no underscore, <=64
/// chars; members need a non-empty agent and role.
#[tokio::test]
async fn team_name_charset_and_member_fields_are_validated() {
    let h = Harness::new().await;
    let mut bad: Vec<(String, serde_json::Value)> = vec![
        ("uppercase".into(), team_named("TeamA")),
        ("underscore".into(), team_named("a_b")),
        ("too long".into(), team_named(&"a".repeat(65))),
        (
            "empty agent".into(),
            json!({"name": "ok-a", "captain": "m1", "members": [member("m1", "", "r")]}),
        ),
        (
            "empty role".into(),
            json!({"name": "ok-r", "captain": "m1", "members": [member("m1", "act", "")]}),
        ),
    ];
    for (label, body) in bad.drain(..) {
        let (status, reply) = h.req(Method::POST, "/api/teams", Some(body)).await;
        assert_eq!(status, 400, "{label}: {reply}");
        assert!(
            reply["error"]
                .as_str()
                .unwrap()
                .contains("unique member id, agent and role"),
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
            Some(json!({"name": "demo", "captain": "m1", "members": [
                member("m1", "act", "captain"), member("m2", "plan", "advisor")]})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = h
        .req(
            Method::POST,
            "/api/teams",
            Some(json!({"name": "demo", "captain": "m9",
                "members": [member("m9", "explore", "solo")]})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["captain"], json!("m9"));

    let (status, body) = h.req(Method::GET, "/api/teams", None).await;
    assert_eq!(status, 200, "{body}");
    let teams = body["teams"].as_array().unwrap();
    assert_eq!(teams.len(), 1, "{body}");
    assert_eq!(teams[0]["captain"], json!("m9"));
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
            &json!({"name": "system", "captain": "m1", "members": [member("m1", "a", "r")]}),
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

fn python_step(name: &str, depends_on: serde_json::Value) -> serde_json::Value {
    json!({"name": name, "depends_on": depends_on, "kind": {"type": "python", "code": "x=1"}})
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
            json!({"name": "bad", "steps": [python_step("a", json!([])), python_step("a", json!([]))]}),
            "duplicate step name",
        ),
        (
            "dependency cycle",
            json!({"name": "bad", "steps": [
                python_step("a", json!(["b"])), python_step("b", json!(["a"]))]}),
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
            Some(json!({"name": "bare-demo", "steps": [python_step("only", json!([]))]})),
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

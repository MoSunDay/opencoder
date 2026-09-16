//! O3 — the role gate over the real control plane: a freshly minted
//! non-admin token may read identity and executions, may submit and
//! command OPERATOR executions end to end, and is refused everywhere
//! else with the documented 403s. `/api/users` stays admin-only.

use crate::support::fleet_proc::Fleet;
use crate::support::http_util::wait_until;
use crate::support::llm_stub::LlmStub;
use serde_json::{json, Value};

/// Poll one execution until `idle`, authenticated as `token`.
fn wait_idle_as(fleet: &Fleet, id: &str, token: &str) -> Value {
    let log = &fleet.log;
    wait_until(log, &format!("idle status for {id} (as user)"), 180, || {
        let (status, body) =
            fleet.http_as("GET", &format!("/api/executions/{id}"), token, &json!({}));
        (status == 200 && body["execution"]["status"] == json!("idle")).then_some(body)
    })
}

#[test]
fn non_admin_role_gates_the_surface() {
    let stub = LlmStub::spawn_text(&["gated-reply"]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "op-gate-node");

    // The admin mints a plain `user` token through the real admin API.
    let (status, body) = fleet.http(
        "POST",
        "/api/users",
        &json!({"name": "op-e2e-user", "role": "user"}),
    );
    assert_eq!(status, 200, "create user: {body}");
    let token = body["token"]
        .as_str()
        .expect("wire token returned once")
        .to_string();
    assert!(token.starts_with("oc_"), "token shape: {token}");
    assert_eq!(body["user"]["role"], "user");

    // Identity probe through the new token.
    let (status, me) = fleet.http_as("GET", "/api/me", &token, &json!({}));
    assert_eq!(status, 200, "me: {me}");
    assert_eq!(me["name"], "op-e2e-user");
    assert_eq!(me["role"], "user");

    // Admin-only surface refused for the non-admin token.
    let (status, body) = fleet.http_as("GET", "/api/users", &token, &json!({}));
    assert_eq!(status, 403, "users list: {body}");
    assert_eq!(body["error"], "role 'user' may not access this endpoint");
    let (status, body) = fleet.http_as(
        "POST",
        "/api/users",
        &token,
        &json!({"name": "nope", "role": "user"}),
    );
    assert_eq!(status, 403);
    assert_eq!(body["error"], "role 'user' may not access this endpoint");

    // Positive case: the non-admin CAN run an operator execution end to
    // end (submit, inspect, read the session).
    let (status, body) = fleet.http_as(
        "POST",
        "/api/executions",
        &token,
        &json!({"id": "operator-e2e-gated", "kind":"operator", "node_id": fleet.node_id(), "input":{"prompt": "gate probe"}}),
    );
    assert_eq!(status, 202, "non-admin operator create: {body}");
    let doc = wait_idle_as(&fleet, "operator-e2e-gated", &token);
    assert_eq!(doc["execution"]["status"], "idle");
    assert_eq!(doc["result"], json!({"session_id": "operator-e2e-gated"}));
    let (status, detail) = fleet.http_as(
        "GET",
        "/api/executions/operator-e2e-gated/messages",
        &token,
        &json!({}),
    );
    assert_eq!(status, 200, "non-admin transcript read: {detail}");
    assert!(
        detail["chunks"]
            .as_array()
            .expect("message chunks")
            .iter()
            .any(|chunk| {
                chunk["role"] == "assistant"
                    && chunk["encoding"] == "base64"
                    && chunk["total_bytes"].as_u64().is_some_and(|size| size > 0)
                    && !chunk["bytes_b64"].as_str().unwrap_or_default().is_empty()
            }),
        "assistant transcript chunk: {detail}"
    );
    // The public read API returns encoded chunks. Verify the fixture's exact
    // reply through the native session projection as well as the user's
    // access to the nonempty assistant chunk above.
    let (status, session) = fleet.http("GET", "/api/sessions/operator-e2e-gated", &json!({}));
    assert_eq!(status, 200, "native transcript read: {session}");
    assert!(session["messages"].to_string().contains("gated-reply"));

    // Non-operator submissions are refused for non-admins: the role gate
    // lets POST /api/executions through, then the kind check rejects with
    // the documented text.
    for kind in ["dag", "agent", "team"] {
        let (status, body) = fleet.http_as(
            "POST",
            "/api/executions",
            &token,
            &json!({"id": format!("{kind}-forbidden"), "kind": kind}),
        );
        assert_eq!(status, 403, "{kind} submit as user: {body}");
        assert_eq!(
            body["error"],
            "non-admin roles may only submit operator executions"
        );
    }
}

#[test]
fn admin_keeps_everything() {
    let stub = LlmStub::spawn_text(&[]);
    let tmp = tempfile::tempdir().unwrap();
    let fleet = Fleet::spawn_with_config(tmp.path(), stub.port(), json!({}), "op-gate-admin");
    // The bootstrap admin token sees itself and the users surface.
    let (status, me) = fleet.http("GET", "/api/me", &json!({}));
    assert_eq!(status, 200);
    assert_eq!(me["role"], "admin");
    let (status, body) = fleet.http("GET", "/api/users", &json!({}));
    assert_eq!(status, 200, "admin users list: {body}");
    assert!(
        body["users"]
            .as_array()
            .expect("users")
            .iter()
            .any(|u| u["role"] == "admin"),
        "bootstrap admin listed: {body}"
    );
}

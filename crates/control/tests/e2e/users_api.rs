//! Platform-user administration + role gate over the real router: /api/me,
//! admin user CRUD, the non-admin read/launch profile, and the operator-only
//! submission/command rules for non-admin roles.

use reqwest::Method;
use serde_json::{json, Value};

use crate::support::{http::Harness, TOKEN};

/// JSON request with an explicit bearer; returns (status, parsed body).
async fn auth(
    h: &Harness,
    method: Method,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (reqwest::StatusCode, Value) {
    let resp = h.req_raw(method, path, body, Some(token)).await;
    let status = resp.status();
    let bytes = resp.bytes().await.unwrap();
    let parsed = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or(json!({}))
    };
    (status, parsed)
}

/// Bearer-less request.
async fn anon(h: &Harness, method: Method, path: &str) -> reqwest::StatusCode {
    h.req_raw(method, path, None, None).await.status()
}

#[tokio::test]
async fn me_reports_the_seed_admin_and_rejects_missing_tokens() {
    let h = Harness::new().await;
    let (status, body) = auth(&h, Method::GET, "/api/me", TOKEN, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({"name": "admin", "role": "admin"}));

    let status = anon(&h, Method::GET, "/api/me").await;
    assert_eq!(status, 401);
    let (status, _) = auth(&h, Method::GET, "/api/me", "wrong-token", None).await;
    assert_eq!(status, 401);
}

/// Create → probe with the issued token → list → revoke → token dead.
#[tokio::test]
async fn admin_creates_lists_and_revokes_users() {
    let h = Harness::new().await;
    let (status, body) = auth(
        &h,
        Method::POST,
        "/api/users",
        TOKEN,
        Some(json!({"name": "alice", "role": "user"})),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["user"]["name"], json!("alice"));
    assert_eq!(body["user"]["role"], json!("user"));
    let token = body["token"].as_str().unwrap().to_string();
    // The plaintext is returned exactly once and in the `oc_` wire format.
    assert!(
        token.starts_with("oc_"),
        "plaintext oc_ token returned exactly once: {token}"
    );
    assert!(
        token.len() > "oc_".len(),
        "plaintext token carries randomness"
    );

    // The issued token authenticates as its user.
    let (status, me) = auth(&h, Method::GET, "/api/me", &token, None).await;
    assert_eq!(status, 200, "{me}");
    assert_eq!(me, json!({"name": "alice", "role": "user"}));

    // Duplicate names collide with 409; unknown roles and bad names 400.
    let (status, _) = auth(
        &h,
        Method::POST,
        "/api/users",
        TOKEN,
        Some(json!({"name": "alice", "role": "root"})),
    )
    .await;
    assert_eq!(status, 409);
    let (status, _) = auth(
        &h,
        Method::POST,
        "/api/users",
        TOKEN,
        Some(json!({"name": "x y", "role": "user"})),
    )
    .await;
    assert_eq!(status, 400);
    let (status, _) = auth(
        &h,
        Method::POST,
        "/api/users",
        TOKEN,
        Some(json!({"name": "ok", "role": "boss"})),
    )
    .await;
    assert_eq!(status, 400);

    // Listing shows the user without any token material.
    let (status, list) = auth(&h, Method::GET, "/api/users", TOKEN, None).await;
    assert_eq!(status, 200, "{list}");
    let users = list["users"].as_array().unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0]["name"], json!("alice"));
    assert!(users[0].get("token_hash").is_none());
    assert!(users[0].get("token").is_none());

    // Revocation kills the credential; double delete is 404.
    let (status, body) = auth(&h, Method::DELETE, "/api/users/alice", TOKEN, None).await;
    assert_eq!(status, 200, "{body}");
    let (status, _) = auth(&h, Method::GET, "/api/me", &token, None).await;
    assert_eq!(status, 401);
    let (status, _) = auth(&h, Method::DELETE, "/api/users/alice", TOKEN, None).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn delete_protections_cover_self_and_the_last_admin() {
    let h = Harness::new().await;
    // The seed identity is "admin": self-delete is refused before lookup.
    let (status, body) = auth(&h, Method::DELETE, "/api/users/admin", TOKEN, None).await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["error"],
        json!("cannot delete the caller's own account")
    );

    // A lone table admin cannot be removed…
    auth(
        &h,
        Method::POST,
        "/api/users",
        TOKEN,
        Some(json!({"name": "boss", "role": "admin"})),
    )
    .await;
    let (status, body) = auth(&h, Method::DELETE, "/api/users/boss", TOKEN, None).await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], json!("cannot delete the last admin"));

    // …but a second admin unlocks the removal.
    auth(
        &h,
        Method::POST,
        "/api/users",
        TOKEN,
        Some(json!({"name": "boss2", "role": "admin"})),
    )
    .await;
    let (status, _) = auth(&h, Method::DELETE, "/api/users/boss", TOKEN, None).await;
    assert_eq!(status, 200);
}

async fn user_token_h(h: &Harness, name: &str, role: &str) -> String {
    let (status, body) = auth(
        h,
        Method::POST,
        "/api/users",
        TOKEN,
        Some(json!({"name": name, "role": role})),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    body["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn non_admins_get_the_read_and_operator_launch_profile() {
    let h = Harness::new().await;
    for role in ["user", "root"] {
        let token = user_token_h(&h, &format!("nina-{role}"), role).await;
        // Reads allowed…
        let (status, _) = auth(&h, Method::GET, "/api/nodes", &token, None).await;
        assert_eq!(status, 200, "{role} nodes");
        let (status, _) = auth(&h, Method::GET, "/api/executions", &token, None).await;
        assert_eq!(status, 200, "{role} executions");
        // …management and other surfaces stay admin-only.
        let (status, _) = auth(&h, Method::GET, "/api/users", &token, None).await;
        assert_eq!(status, 403, "{role} users");
        let (status, _) = auth(
            &h,
            Method::POST,
            "/api/users",
            &token,
            Some(json!({"name": "x", "role": "user"})),
        )
        .await;
        assert_eq!(status, 403, "{role} create user");
        let (status, _) = auth(&h, Method::GET, "/api/agents", &token, None).await;
        assert_eq!(status, 403, "{role} agents");
        let (status, _) = auth(&h, Method::GET, "/api/brain/capabilities", &token, None).await;
        assert_eq!(status, 403, "{role} brain");
        let (status, _) = auth(
            &h,
            Method::POST,
            "/api/nodes/node-e2e/maintenance",
            &token,
            Some(json!({})),
        )
        .await;
        assert_eq!(status, 403, "{role} maintenance");
    }
}

#[tokio::test]
async fn non_admins_submit_and_command_operator_executions_only() {
    let h = Harness::new().await;
    let token = user_token_h(&h, "op-user", "user").await;

    // operator submissions are accepted and pinned to the requested node.
    let (status, receipt) = auth(
        &h,
        Method::POST,
        "/api/executions",
        &token,
        Some(json!({
            "id": "operator-nina-1",
            "kind": "operator",
            "node_id": "node-e2e",
            "input": {"prompt": "hi"}
        })),
    )
    .await;
    assert_eq!(status, 202, "{receipt}");
    assert_eq!(receipt["node_id"], json!("node-e2e"));
    assert!(h
        .node
        .journal_ids()
        .contains(&"operator-nina-1".to_string()));

    // agent (or any other kind) submissions are refused before placement.
    let (status, body) = auth(
        &h,
        Method::POST,
        "/api/executions",
        &token,
        Some(json!({"id": "agent-nina-1", "kind": "agent", "input": {"prompt": "hi"}})),
    )
    .await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(
        body["error"],
        json!("non-admin roles may only submit operator executions")
    );

    // Reading the operator execution works; the id must keep the operator-
    // prefix (the scripted node supplies the inspect body).
    h.node
        .set_inspect("operator-nina-1", json!({"status": "running"}));
    let (status, body) = auth(
        &h,
        Method::GET,
        "/api/executions/operator-nina-1",
        &token,
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], json!("running"));

    // Commands on the operator execution pass the kind rule (the node's
    // scripted 400 for an unknown command is the passthrough proof).
    let (status, body) = auth(
        &h,
        Method::POST,
        "/api/executions/operator-nina-1/commands",
        &token,
        Some(json!({"action": "prompt", "input": {"prompt": "more"}})),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], json!("unknown execution command"));

    // An admin-owned agent execution stays untouchable for commands.
    let (status, _) = auth(
        &h,
        Method::POST,
        "/api/executions",
        TOKEN,
        Some(json!({"id": "agent-admin-1", "kind": "agent", "input": {"prompt": "hi"}})),
    )
    .await;
    assert_eq!(status, 202);
    let (status, body) = auth(
        &h,
        Method::POST,
        "/api/executions/agent-admin-1/commands",
        &token,
        Some(json!({"action": "prompt", "input": {"prompt": "more"}})),
    )
    .await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(
        body["error"],
        json!("non-admin roles may only command operator executions")
    );
}

/// Old nodes never registered `operator`; since PROTOCOL_VERSION is not
/// bumped, an unregistered kind must fail closed through the regular
/// eligibility gate (same 503 shape as the unserved maintenance kind).
#[tokio::test]
async fn operator_kind_without_an_eligible_node_fails_closed() {
    let h = Harness::new().await;
    let (status, body) = auth(
        &h,
        Method::POST,
        "/api/executions",
        TOKEN,
        Some(json!({
            "id": "operator-ghost-1",
            "kind": "operator",
            "node_id": "node-ghost",
            "input": {"prompt": "hi"}
        })),
    )
    .await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(
        body["error"],
        json!("no ready online node can accept this execution")
    );
    assert!(!h
        .node
        .journal_ids()
        .contains(&"operator-ghost-1".to_string()));
}

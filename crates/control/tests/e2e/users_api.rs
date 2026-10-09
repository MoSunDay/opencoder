//! Real-router coverage for live roles, independent credentials and startup admin.
use crate::support::{http::Harness, TOKEN};
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};

async fn auth(
    h: &Harness,
    method: Method,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let response = h.req_raw(method, path, body, Some(token)).await;
    let status = response.status();
    (status, response.json().await.unwrap_or(json!({})))
}
async fn create_user(h: &Harness, name: &str, role: &str) {
    let (status, body) = auth(
        h,
        Method::POST,
        "/api/users",
        TOKEN,
        Some(json!({"name":name,"role":role})),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body.get("token").is_none());
}
async fn issue(h: &Harness, name: &str) -> Value {
    let (status, body) = auth(
        h,
        Method::POST,
        "/api/tokens",
        TOKEN,
        Some(json!({"user_name":name,"name":"CLI"})),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(body["token"].as_str().unwrap().starts_with("oc_"));
    body
}

#[tokio::test]
async fn startup_admin_is_protected_and_missing_credentials_fail() {
    let h = Harness::new().await;
    let (status, body) = auth(&h, Method::GET, "/api/me", TOKEN, None).await;
    assert_eq!(status, 200);
    assert_eq!(body, json!({"name":"admin","role":"admin"}));
    assert_eq!(
        h.req_raw(Method::GET, "/api/me", None, None).await.status(),
        401
    );
    assert_eq!(auth(&h, Method::GET, "/api/me", "wrong", None).await.0, 401);
    for role in ["admin", "user", "root", "unknown"] {
        assert_eq!(
            auth(
                &h,
                Method::POST,
                "/api/users",
                TOKEN,
                Some(json!({"name":"other","role":role}))
            )
            .await
            .0,
            400
        );
    }
    assert_eq!(
        auth(&h, Method::DELETE, "/api/users/admin", TOKEN, None)
            .await
            .0,
        400
    );
    assert_eq!(
        auth(
            &h,
            Method::PATCH,
            "/api/users/admin",
            TOKEN,
            Some(json!({"role":"viewer"}))
        )
        .await
        .0,
        400
    );
    assert_eq!(
        auth(
            &h,
            Method::POST,
            "/api/tokens",
            TOKEN,
            Some(json!({"user_name":"admin","name":"extra"}))
        )
        .await
        .0,
        400
    );
}

#[tokio::test]
async fn credentials_are_independent_and_inherit_live_user_roles() {
    let h = Harness::new().await;
    create_user(&h, "alice", "viewer").await;
    assert_eq!(
        auth(
            &h,
            Method::POST,
            "/api/users",
            TOKEN,
            Some(json!({"name":"alice","role":"editor"}))
        )
        .await
        .0,
        409
    );
    let first = issue(&h, "alice").await;
    let second = issue(&h, "alice").await;
    let a = first["token"].as_str().unwrap();
    let b = second["token"].as_str().unwrap();
    assert_ne!(a, b);
    assert!(first["metadata"]["expires_at"].is_null());
    for token in [a, b] {
        assert_eq!(
            auth(&h, Method::GET, "/api/me", token, None).await.1["role"],
            "viewer"
        );
    }
    assert_eq!(
        auth(
            &h,
            Method::PATCH,
            "/api/users/alice",
            TOKEN,
            Some(json!({"role":"editor"}))
        )
        .await
        .0,
        200
    );
    for token in [a, b] {
        assert_eq!(
            auth(&h, Method::GET, "/api/me", token, None).await.1["role"],
            "editor"
        );
    }
    for path in ["/api/users", "/api/tokens"] {
        let (status, body) = auth(&h, Method::GET, path, TOKEN, None).await;
        assert_eq!(status, 200);
        let text = body.to_string();
        assert!(!text.contains(a) && !text.contains(b) && !text.contains("token_hash"));
    }
    let path = format!("/api/tokens/{}", first["metadata"]["id"].as_str().unwrap());
    assert_eq!(auth(&h, Method::DELETE, &path, TOKEN, None).await.0, 200);
    assert_eq!(auth(&h, Method::GET, "/api/me", a, None).await.0, 401);
    assert_eq!(auth(&h, Method::GET, "/api/me", b, None).await.0, 200);
    assert_eq!(
        auth(&h, Method::DELETE, "/api/users/alice", TOKEN, None)
            .await
            .0,
        200
    );
    assert_eq!(auth(&h, Method::GET, "/api/me", b, None).await.0, 401);
    assert_eq!(
        auth(&h, Method::DELETE, "/api/users/alice", TOKEN, None)
            .await
            .0,
        404
    );
}

#[tokio::test]
async fn viewer_reads_platform_editor_writes_and_both_cannot_administer() {
    let h = Harness::new().await;
    h.put_index(
        "maintenance-private",
        ExecutionKind::Maintenance,
        ExecutionStatus::Idle,
    )
    .await;
    h.put_index("team-public", ExecutionKind::Team, ExecutionStatus::Idle)
        .await;
    for role in ["viewer", "editor"] {
        create_user(&h, role, role).await;
        let issued = issue(&h, role).await;
        let token = issued["token"].as_str().unwrap();
        for path in [
            "/api/nodes",
            "/api/executions",
            "/api/agents",
            "/api/brain/capabilities",
            "/api/project/goals",
        ] {
            let (status, body) = auth(&h, Method::GET, path, token, None).await;
            assert_eq!(status, 200, "{role} {path}: {body}");
        }
        let (_, list) = auth(&h, Method::GET, "/api/executions", token, None).await;
        assert!(!list.to_string().contains("maintenance-private"));
        for path in [
            "/api/users",
            "/api/tokens",
            "/api/executions/maintenance-private",
            "/api/sessions/maintenance-private",
        ] {
            assert_eq!(
                auth(&h, Method::GET, path, token, None).await.0,
                403,
                "{role} {path}"
            );
        }
        for path in [
            "/api/users",
            "/api/tokens",
            "/api/nodes/node-e2e/maintenance",
        ] {
            assert_eq!(
                auth(&h, Method::POST, path, token, Some(json!({}))).await.0,
                403
            );
        }
        let (status, body) = auth(
            &h,
            Method::POST,
            "/api/project/goals",
            token,
            Some(json!({"title":format!("{role} project")})),
        )
        .await;
        assert_eq!(status, if role == "viewer" { 403 } else { 200 }, "{body}");
        for kind in ["agent", "operator"] {
            let (status,body)=auth(&h,Method::POST,"/api/executions",token,Some(json!({"id":format!("{kind}-{role}"),"kind":kind,"node_id":"node-e2e","input":{"prompt":"hello"}}))).await;
            assert_eq!(status, if role == "viewer" { 403 } else { 202 }, "{body}");
        }
        // An invalid platform payload reaches validation for editors, but is denied before routing for viewers.
        for kind in ["dag", "team", "todos", "brain"] {
            let (status, body) = auth(
                &h,
                Method::POST,
                "/api/executions",
                token,
                Some(json!({"id":format!("{kind}-{role}"),"kind":kind})),
            )
            .await;
            if role == "viewer" {
                assert_eq!(status, 403);
            } else {
                assert_ne!(status, 403, "{body}");
            }
        }
        assert_eq!(
            auth(
                &h,
                Method::POST,
                "/api/executions",
                token,
                Some(json!({"id":"maintenance-forbidden","kind":"maintenance"}))
            )
            .await
            .0,
            403
        );
    }
}

#[tokio::test]
async fn token_validation_rejects_expired_and_missing_owners() {
    let h = Harness::new().await;
    create_user(&h, "alice", "viewer").await;
    for body in [
        json!({"user_name":"alice","name":"","expires_at":null}),
        json!({"user_name":"alice","name":"expired","expires_at":1}),
    ] {
        assert_eq!(
            auth(&h, Method::POST, "/api/tokens", TOKEN, Some(body))
                .await
                .0,
            400
        );
    }
    assert_eq!(
        auth(
            &h,
            Method::POST,
            "/api/tokens",
            TOKEN,
            Some(json!({"user_name":"missing","name":"x"}))
        )
        .await
        .0,
        404
    );
}

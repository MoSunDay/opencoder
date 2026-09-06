//! Overflow coverage for the compat surface: models/skills error paths and
//! fleet-eligibility gating (including a nodeless control plane), dialogs
//! rows (happy/degraded/node-scoped) and task cancel error ownership.

use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use reqwest::Method;
use serde_json::json;
use std::time::Duration;

use crate::support::{Harness, TOKEN};

#[tokio::test]
async fn models_skills_error_paths_and_frozen_fleet_gate() {
    let h = Harness::new().await;
    // Ghost node pin: the hub refuses before any node sees the call.
    let (status, body) = h.req(Method::GET, "/api/skills?node_id=ghost", None).await;
    assert_eq!(status, 503, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("offline"),
        "{body}"
    );

    // Unseeded maintenance action: the node's 400 passes through verbatim.
    let (status, body) = h.req(Method::GET, "/api/models", None).await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], json!("unknown maintenance operation"));

    // Freezing flips the node's readiness, so auto-selected config reads
    // end up on the eligible-fleet 503 (explicit node pins still work).
    h.node
        .set_maintenance("models", 200, json!({"models": [{"id": "glm-5.2"}]}));
    let (status, _) = h.req(Method::POST, "/api/admin/drain", None).await;
    assert_eq!(status, 200);
    let mut gated = None;
    for _ in 0..250 {
        let (status, body) = h.req(Method::GET, "/api/models", None).await;
        if status == 503 {
            gated = Some(body);
            break;
        }
        assert_eq!(status, 200, "{body}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let body = gated.expect("frozen fleet must stop serving auto-selected models");
    assert_eq!(
        body["error"],
        json!("no online node for configuration query")
    );
    let (status, body) = h
        .req(Method::GET, "/api/models?node_id=node-e2e", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["models"][0]["id"], json!("glm-5.2"));
}

#[tokio::test]
async fn nodeless_control_plane_answers_config_queries_with_503() {
    // Bare server (no node ever connects): config reads fail fast.
    let dir = tempfile::tempdir().unwrap();
    let state =
        opencoder_control::new_state(dir.path().join("work"), dir.path().join("data"), None)
            .await
            .unwrap();
    let app = opencoder_control::build_app(state, Some(TOKEN.into()), false);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for path in ["/api/models", "/api/skills"] {
        let resp = client
            .get(format!("{base}{path}"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 503, "{path}");
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(
            body["error"],
            json!("no online node for configuration query"),
            "{path}"
        );
    }
    server.abort();
}

#[tokio::test]
async fn dialogs_rows_are_happy_degraded_and_node_scoped() {
    let h = Harness::new().await;
    h.put_index("agent-dlg-2", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    h.node.set_command(
        "agent-dlg-2",
        "summary",
        200,
        json!({
            "id": "agent-dlg-2", "title": "chat", "status": "idle",
            "created_at": 7, "updated_at": 9
        }),
    );
    // Degraded row: index only, no seeded summary command.
    h.put_index(
        "agent-dlg-3",
        ExecutionKind::Agent,
        ExecutionStatus::Running,
    )
    .await;

    let (status, body) = h
        .req(Method::GET, "/api/nodes/node-e2e/dialogs", None)
        .await;
    assert_eq!(status, 200, "{body}");
    let rows = body["dialogs"].as_array().unwrap();
    let happy = rows
        .iter()
        .find(|r| r["session_id"] == json!("agent-dlg-2"))
        .expect("happy row");
    assert_eq!(happy["title"], json!("chat"));
    assert_eq!(happy["status"], json!("idle"));
    assert_eq!(happy["first_created_at"], json!(7));
    assert_eq!(happy["last_created_at"], json!(9));
    let degraded = rows
        .iter()
        .find(|r| r["session_id"] == json!("agent-dlg-3"))
        .expect("degraded row");
    assert_eq!(degraded["title"], json!(null));
    assert_eq!(degraded["status"], json!("running"));
    assert!(degraded["first_created_at"].as_i64().unwrap_or(0) > 0);
    assert_eq!(degraded["last_created_at"], json!(null));
    assert_eq!(
        degraded["detail_error"]["error"],
        json!("unknown execution command")
    );

    // Dialogs are scoped by index.node_id, not by the connected fleet.
    let (status, body) = h
        .req(Method::GET, "/api/nodes/node-other/dialogs", None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["dialogs"], json!([]));
}

#[tokio::test]
async fn task_cancel_errors_are_control_or_node_owned() {
    let h = Harness::new().await;
    // Unknown execution: control-plane owned 409, no node roundtrip.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/nodes/node-e2e/tasks/agent-none/cancel",
            None,
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(
        body["error"],
        json!("execution belongs to a different node")
    );

    // Known execution: the node's conflict reply passes through verbatim.
    h.put_index(
        "agent-cancel-1",
        ExecutionKind::Agent,
        ExecutionStatus::Done,
    )
    .await;
    h.node.set_command(
        "agent-cancel-1",
        "cancel",
        409,
        json!({"error": "already done"}),
    );
    let (status, body) = h
        .req(
            Method::POST,
            "/api/nodes/node-e2e/tasks/agent-cancel-1/cancel",
            None,
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body, json!({"error": "already done"}));
}

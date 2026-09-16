//! The chat page's kind=agent face: `POST /api/sessions` accepts
//! `kind: "agent"` (default stays operator), the id prefix follows the kind,
//! unknown kinds are client errors, the oversized `how_append` payload is
//! refused at admission, and GET /api/sessions lists operator AND agent
//! sessions — never dag/team/system families.

use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};
use reqwest::Method;
use serde_json::json;

use crate::support::Harness;

#[tokio::test]
async fn sessions_create_maps_kind_and_prefix() {
    let h = Harness::new().await;

    // Default (and explicit operator) keeps the operator prefix/kind.
    for body in [
        json!({"agent": "act", "prompt": "hi"}),
        json!({"kind": "operator", "agent": "act", "prompt": "hi"}),
    ] {
        let (status, body) = h.req(Method::POST, "/api/sessions", Some(body)).await;
        assert_eq!(status, 200, "{body}");
        let id = body["id"].as_str().unwrap().to_string();
        assert!(id.starts_with("operator-"), "{id}");
        assert_eq!(body["execution"]["kind"], json!("operator"));
    }

    // kind=agent launches the same session executor on the agent kind.
    // The whole body IS the execution input, so `prompt`/`how_append` ride
    // at the top level (exactly like the operator chat facade).
    let (status, body) = h
        .req(
            Method::POST,
            "/api/sessions",
            Some(json!({
                "kind": "agent",
                "agent": "act",
                "prompt": "hi",
                "how_append": "shared note"
            })),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let id = body["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("agent-"), "{id}");
    assert_eq!(body["execution"]["kind"], json!("agent"));
    assert_eq!(body["execution"]["node_id"], json!("node-e2e"));

    // Unknown kinds are refused before any node call.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/sessions",
            Some(json!({"kind": "team", "agent": "act"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("use operator or agent"),
        "{body}"
    );
    // Only operator/agent ids ever reached the node journal.
    assert!(h
        .node
        .journal_ids()
        .iter()
        .all(|id| id.starts_with("operator-") || id.starts_with("agent-")));

    // A caller-supplied id with a mismatched prefix falls to the generic
    // validate message (kind won, prefix lost).
    let (status, body) = h
        .req(
            Method::POST,
            "/api/sessions",
            Some(json!({"kind": "agent", "id": "operator-mixed-1", "agent": "act"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("id must start with agent-"),
        "{body}"
    );

    // The oversized how_append budget is enforced before placement.
    let oversized = "x".repeat(8 * 1024 + 1);
    let (status, body) = h
        .req(
            Method::POST,
            "/api/sessions",
            Some(json!({"kind": "agent", "agent": "act", "prompt": "hi", "how_append": oversized})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["error"],
        json!("how_append exceeds 8192 bytes (got 8193)")
    );
    // Exactly at the budget is accepted.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/sessions",
            Some(json!({
                "kind": "agent",
                "agent": "act",
                "id": "agent-how-limit-1",
                "prompt": "hi",
                "how_append": "x".repeat(8 * 1024)
            })),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["execution"]["kind"], json!("agent"));
}

#[tokio::test]
async fn sessions_list_merges_operator_and_agent_kinds_only() {
    let h = Harness::new().await;
    h.put_index(
        "operator-list-1",
        ExecutionKind::Operator,
        ExecutionStatus::Idle,
    )
    .await;
    h.put_index("agent-list-1", ExecutionKind::Agent, ExecutionStatus::Idle)
        .await;
    // Every other family stays off the chat surface.
    h.put_index("dag-list-1", ExecutionKind::Dag, ExecutionStatus::Done)
        .await;
    h.put_index("team-list-1", ExecutionKind::Team, ExecutionStatus::Running)
        .await;
    h.put_index(
        "maintenance-list-1",
        ExecutionKind::Maintenance,
        ExecutionStatus::Done,
    )
    .await;

    let (status, body) = h.req(Method::GET, "/api/sessions", None).await;
    assert_eq!(status, 200, "{body}");
    let ids: Vec<String> = body["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["id"].as_str().map(str::to_string))
        .collect();
    assert!(ids.contains(&"operator-list-1".to_string()), "{ids:?}");
    assert!(ids.contains(&"agent-list-1".to_string()), "{ids:?}");
    assert!(
        !ids.iter().any(|id| {
            id.starts_with("dag-")
                || id.starts_with("team-")
                || id.starts_with("maintenance-")
                || id.starts_with("todos-")
        }),
        "foreign families leaked into the chat list: {ids:?}"
    );
    // The merged rows keep the durable `created_at DESC` ordering.
    let created: Vec<i64> = body["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["created_at"].as_i64())
        .collect();
    let mut sorted = created.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(created, sorted, "rows must stay created_at-descending");
}

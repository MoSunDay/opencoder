//! POST /api/executions submit branches: node_id validation and pinning,
//! id conflict passthrough from the node's durable journal, unserved kinds
//! and catalog target resolution on the generic route.

use reqwest::Method;
use serde_json::{json, Value};

use crate::support::Harness;

async fn submit(h: &Harness, body: Value) -> (reqwest::StatusCode, Value) {
    h.req(Method::POST, "/api/executions", Some(body)).await
}

#[tokio::test]
async fn submit_validates_node_id_and_honors_the_pin() {
    let h = Harness::new().await;
    // Malformed pin is rejected before placement.
    let (status, body) = submit(
        &h,
        json!({"id": "agent-pin-1", "kind": "agent", "input": {"prompt": "hi"}, "node_id": "bad/id"}),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], json!("invalid node_id"));

    // A well-formed pin to an unknown node leaves no eligible node.
    let (status, body) = submit(
        &h,
        json!({"id": "agent-pin-1", "kind": "agent", "input": {"prompt": "hi"}, "node_id": "node-ghost"}),
    )
    .await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(
        body["error"],
        json!("no ready online node can accept this execution")
    );
    assert!(h.node.journal_ids().is_empty(), "nothing reached the node");

    // A pin to the live node lands the execution on exactly that node.
    let (status, receipt) = submit(
        &h,
        json!({"id": "agent-pin-1", "kind": "agent", "input": {"prompt": "hi"}, "node_id": "node-e2e"}),
    )
    .await;
    assert_eq!(status, 202, "{receipt}");
    assert_eq!(receipt["id"], json!("agent-pin-1"));
    assert_eq!(receipt["node_id"], json!("node-e2e"));
}

#[tokio::test]
async fn submit_conflict_for_same_id_with_different_input_passes_through() {
    let h = Harness::new().await;
    let (status, receipt) = submit(
        &h,
        json!({"id": "agent-conflict-input-1", "kind": "agent", "input": {"prompt": "first"}}),
    )
    .await;
    assert_eq!(status, 202, "{receipt}");

    // The control plane replays the create; the node's durable journal
    // rejects a different input for a known id and the 409 passes through.
    let (status, body) = submit(
        &h,
        json!({"id": "agent-conflict-input-1", "kind": "agent", "input": {"prompt": "second"}}),
    )
    .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(
        body["error"],
        json!("execution id already accepted with different input")
    );
    assert_eq!(h.node.journal_ids(), vec!["agent-conflict-input-1"]);
}

/// MockNode registers agent/dag/team/todos/project; `maintenance` parses from
/// JSON but no node serves it, so placement fails closed with 503.
#[tokio::test]
async fn submit_of_an_unserved_kind_has_no_eligible_node() {
    let h = Harness::new().await;
    let (status, body) = submit(&h, json!({"id": "maintenance-1", "kind": "maintenance"})).await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(
        body["error"],
        json!("no ready online node can accept this execution")
    );
    assert!(h.node.journal_ids().is_empty(), "nothing reached the node");
}

#[tokio::test]
async fn submit_resolves_catalog_targets_on_the_generic_route() {
    let h = Harness::new().await;
    // (request, expected status, expected error substring).
    let cases: [(Value, u16, &str); 6] = [
        // Unknown DAG definition target.
        (
            json!({"id": "dag-cat-1", "kind": "dag", "target": "nope"}),
            404,
            "definition not found",
        ),
        // DAG without a target and without an inline definition.
        (
            json!({"id": "dag-cat-2", "kind": "dag"}),
            400,
            "target required",
        ),
        // The system team is retired.
        (
            json!({"id": "team-cat-1", "kind": "team", "target": "system"}),
            400,
            "system team execution is retired",
        ),
        // Todos targets must be template/version.
        (
            json!({"id": "todos-cat-1", "kind": "todos", "target": "noSlash"}),
            400,
            "target must be template/version",
        ),
        // Share-name traversal in the template target is rejected.
        (
            json!({"id": "todos-cat-2", "kind": "todos", "target": "../x/1"}),
            400,
            "",
        ),
        // Project targets must reference an existing todo.
        (
            json!({"id": "project-cat-ghost", "kind": "project", "target": "cat-ghost"}),
            404,
            "todo not found",
        ),
    ];
    for (request, want_status, want_error) in cases {
        let (status, body) = submit(&h, request).await;
        assert_eq!(status, want_status, "{body}");
        if want_error.is_empty() {
            assert!(
                body["error"].as_str().is_some_and(|e| !e.is_empty()),
                "{body}"
            );
        } else {
            assert!(
                body["error"]
                    .as_str()
                    .is_some_and(|e| e.contains(want_error)),
                "want {want_error:?} in {body}"
            );
        }
    }
    // Every case failed before placement: no journal, no index rows.
    assert!(h.node.journal_ids().is_empty(), "nothing reached the node");
    let (status, body) = h.req(Method::GET, "/api/executions", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["executions"], json!([]), "{body}");
}

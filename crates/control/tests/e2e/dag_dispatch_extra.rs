//! Extra dispatch-surface coverage for the compat DAG/Todos workflows:
//! minted ids, keyed idempotency, prefix/kind/node conflicts, node pinning
//! and the node-side 428 definition-refresh retry.

use opencoder_core::fleet::ExecutionKind;
use opencoder_core::fleet::ExecutionStatus;
use reqwest::Method;
use serde_json::{json, Value};

use crate::support::{Harness, SHARE_GATE};

const SPEC: &str = r#"{"name":"etl-demo","steps":[
    {"name":"fetch","kind":{"type":"wasm","command":"tool.wasm"}},
    {"name":"load","depends_on":["fetch"],"kind":{"type":"wasm","command":"tool.wasm"}}]}"#;

async fn seed_definition(h: &Harness) {
    let spec: Value = serde_json::from_str(SPEC).unwrap();
    let (status, body) = h
        .req(Method::POST, "/api/dag/defs", Some(json!({"spec": spec})))
        .await;
    assert_eq!(status, 200, "{body}");
}

async fn dispatch(h: &Harness, body: Value) -> (reqwest::StatusCode, Value) {
    h.req(Method::POST, "/api/dag/defs/etl-demo/dispatch", Some(body))
        .await
}

/// Omitting `id` mints a prefixed ULID: `dag-<ulid>` for DAG dispatch and
/// `todos-<ulid>` for template runs.
#[tokio::test]
async fn dispatch_mints_prefixed_ulid_ids() {
    let h = Harness::new().await;
    seed_definition(&h).await;
    let (status, body) = dispatch(&h, json!({})).await;
    assert_eq!(status, 202, "{body}");
    let id = body["run_id"].as_str().unwrap();
    assert!(id.starts_with("dag-") && id.len() > 4, "{body}");
    assert_eq!(body["execution"]["id"], json!(id));
    assert_eq!(h.node.journal_ids(), vec![id.to_owned()]);
}

/// Same rule for the todos template surface (scoped share dir: the template
/// store is process-global, so the run rides the shared gate).
fn todos_spec(name: &str) -> Value {
    json!({
        "schema_version": 1,
        "id": format!("wf-{name}"),
        "name": name,
        "objective": "ship it",
        "todos": [{
            "id": "t1", "title": "T1", "requirement_background": "bg",
            "instructions": "do it", "agent": "act",
            "acceptance": {"criteria": "c"},
        }],
        "metadata": {},
    })
}

#[tokio::test]
async fn todos_template_run_mints_a_todos_prefix_id() {
    let _guard = SHARE_GATE.lock().await;
    let root = std::env::temp_dir().join(format!("oc-ctl-e2e-dagdisp-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    opencoder_core::set_share_dir_override(Some(root.clone()));
    let agents = root.join("agents-root");
    std::fs::create_dir_all(&agents).unwrap();
    opencoder_core::agent::set_agents_dir_override(Some(agents));

    let h = Harness::new().await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/todo/templates",
            Some(json!({"name": "auto", "spec": todos_spec("auto")})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = h
        .req(
            Method::POST,
            "/api/todo/templates/auto/v1/run",
            Some(json!({})),
        )
        .await;
    assert_eq!(status, 202, "{body}");
    let id = body["workflow_id"].as_str().unwrap();
    assert!(id.starts_with("todos-") && id.len() > 6, "{body}");
    assert_eq!(body["execution"]["id"], json!(id));
    assert_eq!(h.node.journal_ids(), vec![id.to_owned()]);
}

/// Re-dispatching the same keyed body replays the identical receipt (same
/// created_at) and the node journal holds a single entry.
#[tokio::test]
async fn keyed_dispatch_is_idempotent_across_retries() {
    let h = Harness::new().await;
    seed_definition(&h).await;
    let (status, first) = dispatch(&h, json!({"id": "dag-idem-1"})).await;
    assert_eq!(status, 202, "{first}");
    let (status, second) = dispatch(&h, json!({"id": "dag-idem-1"})).await;
    assert_eq!(status, 202, "{second}");
    assert_eq!(first, second, "receipts must be identical");
    assert!(
        first["execution"]["created_at"].as_i64().unwrap_or(0) > 0,
        "{first}"
    );
    assert_eq!(h.node.journal_ids(), vec!["dag-idem-1"]);
}

/// Ids are kind-scoped: a dag surface rejects a todos-prefixed id (and vice
/// versa) before any definition resolution; durable indexes refuse kind and
/// node reassignment with 409.
#[tokio::test]
async fn dispatch_rejects_foreign_prefixes_and_reassignment() {
    let h = Harness::new().await;
    seed_definition(&h).await;
    let (status, body) = dispatch(&h, json!({"id": "todos-1"})).await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|e| e.contains("id must start with dag-")),
        "{body}"
    );
    // Validated before template lookup, so no template has to exist.
    let (status, body) = h
        .req(
            Method::POST,
            "/api/todo/templates/none/v1/run",
            Some(json!({"id": "dag-1"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|e| e.contains("id must start with todos-")),
        "{body}"
    );

    // Kind conflict: the durable index already says Todos.
    h.put_index("dag-conf-1", ExecutionKind::Todos, ExecutionStatus::Idle)
        .await;
    let (status, body) = dispatch(&h, json!({"id": "dag-conf-1"})).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(
        body["error"],
        json!("execution id is already assigned to another kind")
    );

    // Node conflict: the durable index pins the run to node-e2e.
    h.put_index("dag-conf-2", ExecutionKind::Dag, ExecutionStatus::Idle)
        .await;
    let (status, body) = dispatch(&h, json!({"id": "dag-conf-2", "node_id": "node-ghost"})).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(
        body["error"],
        json!("execution is already assigned to another node")
    );
    assert!(
        h.node.journal_ids().is_empty(),
        "conflicts never reach the node"
    );
}

/// An explicit node_id pins placement; an unknown node has no eligible
/// candidate and fails 503 before any node call.
#[tokio::test]
async fn dispatch_honors_node_pinning() {
    let h = Harness::new().await;
    seed_definition(&h).await;
    let (status, body) = dispatch(&h, json!({"id": "dag-pin-1", "node_id": "node-e2e"})).await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["execution"]["node_id"], json!("node-e2e"));
    assert_eq!(h.node.journal_ids(), vec!["dag-pin-1"]);

    let (status, body) = dispatch(&h, json!({"id": "dag-pin-2", "node_id": "node-ghost"})).await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(
        body["error"],
        json!("no ready online node can accept this execution")
    );
    assert_eq!(h.node.journal_ids(), vec!["dag-pin-1"]);
}

/// A node 428 ("definition moved") makes control re-resolve the definition
/// and retry Create once; the scripted node keeps replying 428, so the
/// surfaced contract is the node reply verbatim, the journal stays empty and
/// the pending run recovers once the node accepts again.
#[tokio::test]
async fn dispatch_surfaces_the_node_428_after_the_definition_retry() {
    let h = Harness::new().await;
    seed_definition(&h).await;
    h.node
        .set_create_reply(428, json!({"error": "definition moved"}));
    let (status, body) = dispatch(&h, json!({"id": "dag-428-1"})).await;
    assert_eq!(status, 428, "{body}");
    assert_eq!(body, json!({"error": "definition moved"}));
    assert!(h.node.journal_ids().is_empty(), "no acceptance happened");

    h.node.clear_create_reply();
    let (status, body) = dispatch(&h, json!({"id": "dag-428-1"})).await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(body["run_id"], json!("dag-428-1"));
    assert_eq!(body["execution"]["node_id"], json!("node-e2e"));
    assert_eq!(h.node.journal_ids(), vec!["dag-428-1"]);
}

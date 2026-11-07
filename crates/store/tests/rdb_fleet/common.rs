//! Shared fixtures for the rdb_fleet suites.
#![allow(dead_code)]
use opencoder_core::fleet::{
    Assignment, CreateExecution, ExecutionIndex, ExecutionKind, ExecutionStatus,
};
use opencoder_store::fleet::FleetStore;
use serde_json::{json, Value};

pub fn index(
    id: &str,
    created_at: i64,
    kind: ExecutionKind,
    status: ExecutionStatus,
) -> ExecutionIndex {
    ExecutionIndex {
        id: id.into(),
        created_at,
        kind,
        node_id: "node-a".into(),
        status,
    }
}

pub fn dag(id: &str, created_at: i64, status: ExecutionStatus) -> ExecutionIndex {
    index(id, created_at, ExecutionKind::Dag, status)
}

pub fn request(id: &str, kind: ExecutionKind, input: Value) -> CreateExecution {
    CreateExecution {
        id: id.into(),
        kind,
        target: Some("coder".into()),
        input,
        node_id: Some("node-a".into()),
    }
}

pub fn assignment(
    index: ExecutionIndex,
    request: CreateExecution,
    definition: Option<Value>,
) -> Assignment {
    Assignment {
        private_context: None,
        runtime: None,
        codex: None,
        index,
        request,
        definition,
    }
}

/// Claim the dispatch receipt and freeze the assignment in one step.
pub async fn prepare(store: &FleetStore, assignment: &Assignment) -> String {
    let key = opencoder_store::fleet::handoff::dispatch_key(&assignment.request).to_owned();
    let fingerprint = opencoder_core::token_hash(&key);
    assert!(store
        .claim_request("execution", &key, &fingerprint)
        .await
        .unwrap());
    store
        .prepare_assignment(assignment, &fingerprint)
        .await
        .unwrap();
    fingerprint
}

pub fn plan_version(id: &str, version: u64, title: &str) -> Value {
    json!({
        "id": id, "version": version,
        "plan": {"schema_version": 4, "title": title, "objective": "test", "nodes": [], "edges": []},
        "changelog": format!("v{version}"), "created_at": version
    })
}

/// Freeze an assignment fixture in the pending phase for the handoff suites.
pub fn frozen(id: &str, kind: ExecutionKind, created_at: i64) -> opencoder_core::fleet::Assignment {
    assignment(
        index(id, created_at, kind, ExecutionStatus::Pending),
        request(id, kind, json!({"prompt": "once"})),
        Some(json!({"version": 1})),
    )
}

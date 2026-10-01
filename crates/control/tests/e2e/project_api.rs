//! Project surface: goal→initiative→todo CRUD, overview aggregation, the
//! plan/act node affinity lifecycle and run views.

use opencoder_core::fleet::{ExecutionKind, ExecutionStatus, RpcReply};
use reqwest::Method;
use serde_json::json;

use crate::support::Harness;

async fn seed_todo(h: &Harness) -> String {
    seed_todo_with_kind(h, None).await
}

/// Same seed but with an explicit executor_kind (e.g. brain pre-resolution).
async fn seed_todo_with_kind(h: &Harness, executor_kind: Option<&str>) -> String {
    let mut body = json!({"title": "T1", "draft": "do it"});
    if let Some(kind) = executor_kind {
        body["executor_kind"] = json!(kind);
    }
    let (status, body) = h.req(Method::POST, "/api/project/todos", Some(body)).await;
    assert_eq!(status, 200, "{body}");
    body["id"].as_str().unwrap().to_string()
}

// The overview reads saved TODOs even when a linked node loses its journal.
// Inspect failures stay on the execution endpoint and do not rewrite TODOs.

#[path = "project_api/suite_1.rs"]
mod suite_1;
#[path = "project_api/suite_2.rs"]
mod suite_2;

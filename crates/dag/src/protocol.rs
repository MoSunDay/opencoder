//! Wire DTOs for the node-side DAG scheduling protocol.
//!
//! Mirrors the role of `opencoder_core::node_protocol`: pure data shared by
//! the server chain (`opencoder-web`) and the agent chain
//! (`opencoder-dag-runtime` / `opencoder-agent`), so a wire-shape change
//! fails to compile on both sides. The SERVER only stores and forwards; all
//! execution happens on the claiming node.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::spec::DagSpec;

/// `POST /api/dag/defs` — create or replace a definition (upsert by `name`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagDefUpsertRequest {
    pub spec: DagSpec,
}

/// Def row as served to browsers / agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagDefView {
    pub id: String,
    pub name: String,
    pub spec: DagSpec,
    pub created_at: i64,
    pub updated_at: i64,
}

/// `POST /api/dag/defs/:id/dispatch { node_id? }` — enqueue a run. A
/// `node_id` pins the run to that node's claim queue; absent means "any
/// node may claim" (`node_id IS NULL` until claimed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagDispatchRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagDispatchResponse {
    pub run_id: String,
}

/// `GET /api/nodes/dag/claim?node_id=` reply body (`200`; `204` = nothing
/// due). The run carries its OWN spec snapshot so later def edits never
/// mutate an in-flight run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagClaimedRun {
    pub run_id: String,
    pub dag_id: String,
    pub spec: DagSpec,
    pub created_at: i64,
}

/// One node-emitted event, uploaded in batches. `kind` is a small closed
/// vocabulary: `run_started | step_started | step_done | run_finished`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagEventIn {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    #[serde(default)]
    pub payload: Value,
    pub at_ms: i64,
}

/// `POST /api/nodes/dag/runs/:rid/events` body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagEventBatch {
    pub run_id: String,
    pub events: Vec<DagEventIn>,
}

/// `POST /api/nodes/dag/runs/:rid/status` body — terminal report
/// (`done | error | cancelled`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagStatusReport {
    pub run_id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Run row as served to browsers (`GET /api/dag/runs`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagRunView {
    pub id: String,
    pub dag_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
}

/// SSE event frame for `GET /api/dag/runs/:rid/events` (replayed from
/// `dag_events` with the row `seq` as the `Last-Event-ID` cursor).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagEventView {
    pub seq: i64,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    pub payload: Value,
    pub at_ms: i64,
}

/// Event kind vocabulary (server validates uploads against this set).
pub const DAG_EVENT_KINDS: [&str; 4] = ["run_started", "step_started", "step_done", "run_finished"];

#[cfg(test)]
mod tests {
    use super::*;

    /// Claim/dispatch bodies stay decodable when optional fields are absent
    /// (older agents, plain stubs) — the same compatibility rule as
    /// `node_protocol`.
    #[test]
    fn optional_fields_default() {
        let d: DagDispatchRequest = serde_json::from_str("{}").unwrap();
        assert!(d.node_id.is_none());
        let ev: DagEventIn =
            serde_json::from_str(r#"{"kind":"step_started","step":"a","at_ms":1}"#).unwrap();
        assert_eq!(ev.payload, Value::Null);
        let r: DagRunView = serde_json::from_str(
            r#"{"id":"r","dag_id":"d","name":"n","status":"pending","created_at":1}"#,
        )
        .unwrap();
        assert!(r.node_id.is_none() && r.finished_at.is_none());
    }

    #[test]
    fn claimed_run_carries_spec_snapshot() {
        let wire = r#"{"run_id":"r1","dag_id":"d1","created_at":5,
            "spec":{"name":"etl","steps":[{"name":"a","kind":{"type":"python","code":"x"}}]}}"#;
        let run: DagClaimedRun = serde_json::from_str(wire).unwrap();
        assert_eq!(run.spec.steps.len(), 1);
        assert_eq!(run.spec.steps[0].name, "a");
    }
}

//! Wire DTOs for the multi-node distributed execution protocol (Phase 2).
//!
//! Pure data + one pure validator: no IO, no state. The control plane
//! (`opencoder-web`) and worker-side binaries share these definitions so a
//! wire-shape change fails to compile on both sides instead of drifting.
//! Serde is the only dependency surface (plus `serde_json::Value` payloads).

use serde::{Deserialize, Serialize};

/// `POST /api/nodes/register` — announce (or re-announce) a worker node.
///
/// Registration is keyed by `name`: a known name keeps its server-issued id,
/// so dispatched tasks never dangle across reconnects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRegisterRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
}

/// Registration success: the stable server-issued node id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRegisterResponse {
    pub node_id: String,
}

/// `POST /api/nodes/:id/heartbeat` reply.
///
/// `server_time_ms` lets the node reconcile clocks; `cancel_task_ids` carries
/// every `cancelling` task this node must abort before its next heartbeat.
/// The request body may be an empty object and carries no data today.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHeartbeatResponse {
    pub server_time_ms: i64,
    pub cancel_task_ids: Vec<String>,
}

/// `POST /api/nodes/:id/tasks` — dispatch one unit of work to a node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDispatchRequest {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Dispatch accepted: task id (queue key) + synthetic session id (event key).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDispatchResponse {
    pub task_id: String,
    pub session_id: String,
}

/// One task handed to a worker by `GET /api/nodes/tasks/claim?node_id=`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimedTask {
    pub task_id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub created_at: i64,
}

/// A single SSE-shaped event uploaded from a worker while executing its task.
///
/// `sse_kind` is the granular event-name string (mirrors the server's
/// `session_events.sse_kind` column); `payload` is the structured data; `ts`
/// is the emitter clock (persisted as-is, the server does not reorder).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeEventIn {
    pub sse_kind: String,
    pub payload: serde_json::Value,
    pub ts: i64,
}

/// Batch upload envelope (`POST /api/nodes/tasks/:tid/events`). An empty
/// batch is legal and appends nothing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeEventBatch {
    pub events: Vec<NodeEventIn>,
}

/// Terminal transition reported by a worker
/// (`POST /api/nodes/tasks/:tid/status`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStatusReport {
    /// Exactly one of `done | error | cancelled` (see [`NodeStatusReport::validate`]).
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl NodeStatusReport {
    /// Strict gate: only the three agreed literals pass. The HTTP layer turns
    /// anything else into a 400 so a typo'd worker cannot invent a status.
    /// (`core` stays dependency-free of store enums; the validated literal is
    /// mapped onto `NodeTaskStatus` by the web layer.)
    pub fn validate(&self) -> bool {
        matches!(self.status.as_str(), "done" | "error" | "cancelled")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Optional fields deserialize when absent and stay absent when serializing
    /// (stable round-trip, no noise keys).
    #[test]
    fn register_request_roundtrip_with_and_without_optionals() {
        let minimal: NodeRegisterRequest =
            serde_json::from_str(r#"{"name":"gpu-1"}"#).expect("minimal must parse");
        assert_eq!(minimal.name, "gpu-1");
        assert!(minimal.version.is_none() && minimal.workdir.is_none());

        let full = NodeRegisterRequest {
            name: "gpu-1".into(),
            version: Some("v9".into()),
            workdir: Some("/w".into()),
        };
        let wire = serde_json::to_string(&full).unwrap();
        let back: NodeRegisterRequest = serde_json::from_str(&wire).unwrap();
        assert_eq!(back.version.as_deref(), Some("v9"));
        assert!(wire.contains("\"version\":\"v9\""));
    }

    /// Dispatch response and claimed task survive a full encode/decode cycle.
    #[test]
    fn dispatch_roundtrip_preserves_ids_and_fields() {
        let resp = NodeDispatchResponse {
            task_id: "01JTASK".into(),
            session_id: "01JSESS".into(),
        };
        let back: NodeDispatchResponse =
            serde_json::from_str(&serde_json::to_string(&resp).unwrap()).unwrap();
        assert_eq!(back.task_id, "01JTASK");
        assert_eq!(back.session_id, "01JSESS");

        let task = ClaimedTask {
            task_id: "t".into(),
            session_id: "s".into(),
            title: Some("T".into()),
            prompt: "do it".into(),
            agent: None,
            model: None,
            created_at: 42,
        };
        let wire = serde_json::to_string(&task).unwrap();
        assert!(
            !wire.contains("\"agent\""),
            "None optionals must be skipped for a stable shape"
        );
        let back: ClaimedTask = serde_json::from_str(&wire).unwrap();
        assert_eq!(back.prompt, "do it");
        assert_eq!(back.created_at, 42);
    }

    /// Heartbeat reply deserializes from a plain-literal wire frame.
    #[test]
    fn heartbeat_response_deserializes() {
        let hb: NodeHeartbeatResponse =
            serde_json::from_str(r#"{"server_time_ms":10,"cancel_task_ids":["a"]}"#).unwrap();
        assert_eq!(hb.cancel_task_ids, vec!["a".to_string()]);
        assert_eq!(hb.server_time_ms, 10);
    }

    /// Event batches (including the empty batch) round-trip losslessly.
    #[test]
    fn event_batch_roundtrip_including_empty() {
        let batch = NodeEventBatch {
            events: vec![NodeEventIn {
                sse_kind: "text_delta".into(),
                payload: serde_json::json!({ "delta": "hi" }),
                ts: 7,
            }],
        };
        let wire = serde_json::to_string(&batch).unwrap();
        let back: NodeEventBatch = serde_json::from_str(&wire).unwrap();
        assert_eq!(back.events.len(), 1);
        assert_eq!(back.events[0].payload["delta"], "hi");

        let empty: NodeEventBatch =
            serde_json::from_str(r#"{"events":[]}"#).expect("empty batch must parse");
        assert!(empty.events.is_empty());
    }

    /// `validate()` is the contract gate: three literals in, everything else out.
    #[test]
    fn status_report_validation_is_exact() {
        for ok in ["done", "error", "cancelled"] {
            let r = NodeStatusReport {
                status: ok.into(),
                error: None,
            };
            assert!(r.validate(), "{ok} must validate");
        }
        for bad in ["Done", "", "complete", "running", "pending", "error "] {
            let r = NodeStatusReport {
                status: bad.into(),
                error: None,
            };
            assert!(!r.validate(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn status_report_error_field_survives_roundtrip() {
        let r = NodeStatusReport {
            status: "error".into(),
            error: Some("boom".into()),
        };
        let back: NodeStatusReport =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back.error.as_deref(), Some("boom"));
    }
}

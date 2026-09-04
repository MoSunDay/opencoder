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
    /// Client-declared reachability address. When absent the server records
    /// the connection's source IP; an explicit value overrides (nodes behind
    /// NAT/proxies may declare their real address).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addr: Option<String>,
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
/// `cancel_run_ids` is the same piggyback for DAG workflow runs (`cancelling`
/// runs the node's workflow executor must abort). `controls` piggybacks
/// queued control tasks ([`ControlTask`]) so a BUSY worker (which never polls
/// claim) still learns about them — the heartbeat is the only channel a busy
/// node is guaranteed to listen on. The request body may be an empty object
/// and carries no data today.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHeartbeatResponse {
    pub server_time_ms: i64,
    pub cancel_task_ids: Vec<String>,
    /// DAG runs this node must abort (usually empty; omitted from the wire).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cancel_run_ids: Vec<String>,
    /// Control tasks handed to this node opportunistically (usually empty).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<ControlTask>,
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
    /// Resume semantics: when present, NO synthetic session is created — the
    /// task binds to this existing session (the console's "continue dialog"
    /// button). The server answers 400 when the session does not exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Dispatch accepted: task id (queue key) + synthetic session id (event key).
/// `status` is the freshly queued state (always `"pending"` today) so the
/// submitter can render a status view without a follow-up fetch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDispatchResponse {
    pub task_id: String,
    pub session_id: String,
    #[serde(default)]
    pub status: String,
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

/// Claim reply envelope: 200 carries a durable task, a control task, or both;
/// `204 No Content` means both were absent. Both fields default so a body of
/// `{}` is a legal (if useless) "nothing to do".
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClaimResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<ClaimedTask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<ControlTask>,
}

/// Kind literal of the only control task today (message-history relay).
pub const TASK_KIND_FETCH_MESSAGES: &str = "fetch_messages";

/// Server→worker control instruction (P3 message relay). Fire-and-forget with
/// a result uploaded back through `POST /api/nodes/:id/control_result`; the
/// server never persists the payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlTask {
    /// Server-issued ULID; matches result to pending browser request.
    pub control_id: String,
    /// Only [`TASK_KIND_FETCH_MESSAGES`] exists today; the literal is data so
    /// the worker can ignore (or the server can add) kinds independently.
    pub kind: String,
    /// The session whose LOCAL transcript the worker must upload.
    pub session_id: String,
}

/// One message of a relayed dialog slice: the raw stored row, decoded as data
/// (blocks stay a raw JSON value so the relay never needs block-level detail).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogMessage {
    /// Persisted per-session message `seq` (the resume boundary unit).
    pub seq: i64,
    /// `system | user | assistant | tool`.
    pub role: String,
    /// Raw stored message blocks JSON (array of content blocks).
    pub blocks: serde_json::Value,
    /// Emitter clock (epoch ms) persisted with the row.
    pub created_at: i64,
}

/// Worker→server result of a [`ControlTask`] (uploaded via
/// `POST /api/nodes/:id/control_result`). `ok = false` carries the reason in
/// `error` and an empty slice; the browser reply then degrades to 502.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchMessagesResult {
    pub control_id: String,
    pub session_id: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Session compaction summary (resume-shaped slice head).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Boundary: only messages with `seq > summary_seq` are included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_seq: Option<i64>,
    pub messages: Vec<DialogMessage>,
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
        assert!(minimal.addr.is_none(), "addr is optional");

        let full = NodeRegisterRequest {
            name: "gpu-1".into(),
            version: Some("v9".into()),
            workdir: Some("/w".into()),
            addr: Some("10.0.0.9".into()),
        };
        let wire = serde_json::to_string(&full).unwrap();
        let back: NodeRegisterRequest = serde_json::from_str(&wire).unwrap();
        assert_eq!(back.version.as_deref(), Some("v9"));
        assert_eq!(back.addr.as_deref(), Some("10.0.0.9"));
        assert!(wire.contains("\"version\":\"v9\""));
    }

    /// Dispatch response and claimed task survive a full encode/decode cycle.
    #[test]
    fn dispatch_roundtrip_preserves_ids_and_fields() {
        let resp = NodeDispatchResponse {
            task_id: "01JTASK".into(),
            session_id: "01JSESS".into(),
            status: "pending".into(),
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

    // ── P3 message-relay DTOs ─────────────────────────────────────────────

    fn sample_control(control_id: &str) -> ControlTask {
        ControlTask {
            control_id: control_id.into(),
            kind: TASK_KIND_FETCH_MESSAGES.into(),
            session_id: "01JSESS".into(),
        }
    }

    fn sample_result(control_id: &str) -> FetchMessagesResult {
        FetchMessagesResult {
            control_id: control_id.into(),
            session_id: "01JSESS".into(),
            ok: true,
            error: None,
            summary: Some("earlier talk".into()),
            summary_seq: Some(7),
            messages: vec![DialogMessage {
                seq: 8,
                role: "assistant".into(),
                blocks: serde_json::json!([{ "kind": "text", "text": "hi" }]),
                created_at: 1234,
            }],
        }
    }

    /// Control task round-trips and its optionals stay absent on the wire.
    #[test]
    fn control_task_roundtrip() {
        let wire = serde_json::to_string(&sample_control("01JCTL")).unwrap();
        assert!(wire.contains("\"kind\":\"fetch_messages\""));
        assert!(!wire.contains("error"), "no extra keys expected: {wire}");
        let back: ControlTask = serde_json::from_str(&wire).unwrap();
        assert_eq!(back.control_id, "01JCTL");
        assert_eq!(back.kind, TASK_KIND_FETCH_MESSAGES);
        assert_eq!(back.session_id, "01JSESS");
    }

    /// FetchMessagesResult survives encode/decode losslessly, including the
    /// summary pair and raw blocks value.
    #[test]
    fn fetch_messages_result_roundtrip() {
        let wire = serde_json::to_string(&sample_result("01JCTL")).unwrap();
        let back: FetchMessagesResult = serde_json::from_str(&wire).unwrap();
        assert_eq!(back.control_id, "01JCTL");
        assert!(back.ok);
        assert_eq!(back.summary.as_deref(), Some("earlier talk"));
        assert_eq!(back.summary_seq, Some(7));
        assert_eq!(back.messages.len(), 1);
        assert_eq!(back.messages[0].seq, 8);
        assert_eq!(back.messages[0].role, "assistant");
        assert_eq!(back.messages[0].blocks[0]["text"], "hi");
    }

    /// Error-shaped result: ok=false + reason, empty slice, no summary keys.
    #[test]
    fn fetch_messages_result_error_shape_roundtrip() {
        let r = FetchMessagesResult {
            control_id: "c".into(),
            session_id: "s".into(),
            ok: false,
            error: Some("session not found".into()),
            summary: None,
            summary_seq: None,
            messages: vec![],
        };
        let wire = serde_json::to_string(&r).unwrap();
        assert!(
            !wire.contains("\"summary\""),
            "None summary must be skipped"
        );
        let back: FetchMessagesResult = serde_json::from_str(&wire).unwrap();
        assert!(!back.ok);
        assert_eq!(back.error.as_deref(), Some("session not found"));
        assert!(back.messages.is_empty());
    }

    /// Claim envelope: task-only, control-only, both, and the empty `{}` body
    /// (defaults make every partial shape parse).
    #[test]
    fn claim_response_envelope_roundtrip() {
        let both = ClaimResponse {
            task: Some(ClaimedTask {
                task_id: "t".into(),
                session_id: "s".into(),
                title: None,
                prompt: "p".into(),
                agent: None,
                model: None,
                created_at: 1,
            }),
            control: Some(sample_control("01JCTL")),
        };
        let back: ClaimResponse =
            serde_json::from_str(&serde_json::to_string(&both).unwrap()).unwrap();
        assert_eq!(back.task.as_ref().unwrap().task_id, "t");
        assert_eq!(back.control.as_ref().unwrap().control_id, "01JCTL");

        let only_control = ClaimResponse {
            task: None,
            control: Some(sample_control("01JCTL")),
        };
        let wire = serde_json::to_string(&only_control).unwrap();
        assert!(!wire.contains("\"task\":"), "None task skipped: {wire}");
        let back: ClaimResponse = serde_json::from_str(&wire).unwrap();
        assert!(back.task.is_none() && back.control.is_some());

        let empty: ClaimResponse = serde_json::from_str("{}").unwrap();
        assert!(empty.task.is_none() && empty.control.is_none());
    }

    /// Heartbeat reply: `controls` is optional on the wire (older servers /
    /// plain stubs stay decodable) and round-trips when populated.
    #[test]
    fn heartbeat_response_controls_default_and_roundtrip() {
        let legacy: NodeHeartbeatResponse =
            serde_json::from_str(r#"{"server_time_ms":1,"cancel_task_ids":[]}"#).unwrap();
        assert!(legacy.controls.is_empty(), "absent controls default empty");

        let hb = NodeHeartbeatResponse {
            server_time_ms: 5,
            cancel_task_ids: vec![],
            cancel_run_ids: vec![],
            controls: vec![sample_control("01JCTL")],
        };
        let wire = serde_json::to_string(&hb).unwrap();
        let back: NodeHeartbeatResponse = serde_json::from_str(&wire).unwrap();
        assert_eq!(back.controls.len(), 1);
        assert_eq!(back.controls[0].kind, TASK_KIND_FETCH_MESSAGES);

        // Empty vec is skipped entirely (no noise on the busy hot path).
        let quiet = serde_json::to_string(&NodeHeartbeatResponse {
            server_time_ms: 5,
            cancel_task_ids: vec![],
            cancel_run_ids: vec![],
            controls: vec![],
        })
        .unwrap();
        assert!(!quiet.contains("controls"), "{quiet}");
    }

    /// Heartbeat reply: `cancel_run_ids` follows the same wire-compat rules
    /// as `controls` — absent decodes empty, populated round-trips, empty is
    /// omitted entirely (older servers / plain stubs stay decodable).
    #[test]
    fn heartbeat_response_cancel_run_ids_default_roundtrip_and_omission() {
        let legacy: NodeHeartbeatResponse =
            serde_json::from_str(r#"{"server_time_ms":1,"cancel_task_ids":["t"]}"#).unwrap();
        assert!(
            legacy.cancel_run_ids.is_empty(),
            "absent cancel_run_ids default empty"
        );

        let hb = NodeHeartbeatResponse {
            server_time_ms: 5,
            cancel_task_ids: vec!["t1".into()],
            cancel_run_ids: vec!["01JDAGRUN".into()],
            controls: vec![],
        };
        let wire = serde_json::to_string(&hb).unwrap();
        assert!(wire.contains("cancel_run_ids"), "{wire}");
        let back: NodeHeartbeatResponse = serde_json::from_str(&wire).unwrap();
        assert_eq!(back.cancel_run_ids, vec!["01JDAGRUN".to_string()]);

        let quiet = serde_json::to_string(&NodeHeartbeatResponse {
            server_time_ms: 5,
            cancel_task_ids: vec![],
            cancel_run_ids: vec![],
            controls: vec![],
        })
        .unwrap();
        assert!(!quiet.contains("cancel_run_ids"), "{quiet}");
    }

    /// Dispatch request: `session_id` is optional (plain dispatch keeps its
    /// old wire shape) and survives when present.
    #[test]
    fn dispatch_request_session_id_optional() {
        let legacy: NodeDispatchRequest = serde_json::from_str(r#"{"prompt":"hi"}"#).unwrap();
        assert!(legacy.session_id.is_none());
        assert!(legacy.title.is_none());

        let full: NodeDispatchRequest =
            serde_json::from_str(r#"{"prompt":"hi","session_id":"01JSESS"}"#).unwrap();
        assert_eq!(full.session_id.as_deref(), Some("01JSESS"));
    }
}

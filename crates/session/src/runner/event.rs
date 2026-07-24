use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use opencoder_core::Message;
use opencoder_store::EventKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionEvent {
    TextDelta(String),
    ReasoningDelta(String),
    ToolStart {
        id: String,
        name: String,
        input: Value,
    },
    ToolEnd {
        id: String,
        name: String,
        output: String,
        is_error: bool,
    },
    AgentSwitch(String),
    Compaction(String),
    Status(String),
    /// A subagent (task tool) started. `child_session_id` is the child's
    /// session for loading its transcript from the store.
    SubagentStart {
        id: String,
        kind: String,
        prompt: String,
        child_session_id: String,
    },
    /// A subagent finished. `cancelled` is set when the run was interrupted
    /// (shared cancel token) before producing a real result; the parent
    /// tool_use is left open in that case to be replayed on the next turn.
    SubagentEnd {
        id: String,
        ok: bool,
        #[serde(default)]
        cancelled: bool,
        summary: String,
    },
    /// A child event from a running subagent, tagged with the tool-call id so
    /// the TUI can route it into the subagent's foldable block.
    SubagentChild {
        id: String,
        ev: Box<SessionEvent>,
    },
    /// Emitted after compaction rewrites the transcript. Carries the new
    /// message list so display surfaces can rebuild their view.
    TranscriptReset(Vec<Message>),
    /// Emitted after plan→act handoff. Carries the plan text (markdown) for the
    /// display layer to render as a read-only card. Paired with a preceding
    /// TranscriptReset that rebuilds the clean view.
    PlanHandoff(String),
    /// A queued follow-up was consumed (drained) at an idle boundary. Carries
    /// the consumed input's row seq so the TUI can drop it from its pending
    /// mirror instead of leaving a stale `[queued]` row until `Done`.
    QueueConsumed {
        seq: i64,
    },
    /// A steered input was consumed (promoted) at a turn boundary. Carries
    /// the consumed input's row seq so the TUI can drop it from its pending
    /// mirror instead of leaving a stale `steer` row until `Done`.
    SteerConsumed {
        seq: i64,
    },
    Done,
    Error(String),
}

impl SessionEvent {
    /// The granular SSE event-name string for this variant.
    /// Single source of truth shared by the web layer (live broadcast +
    /// replay) and the TUI (persistence), so a session driven by either
    /// surface replays identically.
    pub fn sse_kind(&self) -> &'static str {
        match self {
            SessionEvent::TextDelta(_) => "text_delta",
            SessionEvent::ReasoningDelta(_) => "reasoning_delta",
            SessionEvent::ToolStart { .. } => "tool_start",
            SessionEvent::ToolEnd { .. } => "tool_end",
            SessionEvent::AgentSwitch(_) => "agent_switched",
            SessionEvent::Compaction(_) => "compaction",
            SessionEvent::Status(_) => "status",
            SessionEvent::Done => "done",
            SessionEvent::Error(_) => "error",
            SessionEvent::SubagentStart { .. } => "subagent_start",
            SessionEvent::SubagentEnd { .. } => "subagent_end",
            SessionEvent::SubagentChild { .. } => "subagent_child",
            SessionEvent::PlanHandoff(_) => "plan_handoff",
            SessionEvent::TranscriptReset(_) => "transcript_reset",
            SessionEvent::QueueConsumed { .. } => "queue_consumed",
            SessionEvent::SteerConsumed { .. } => "steer_consumed",
        }
    }

    /// The structured JSON payload for this variant, matching the SSE wire
    /// format. Both web and TUI use this for persistence so the replayed
    /// payload shape is identical to the live broadcast.
    pub fn sse_data(&self) -> serde_json::Value {
        match self {
            SessionEvent::TextDelta(t) => serde_json::json!({ "text": t }),
            SessionEvent::ReasoningDelta(r) => serde_json::json!({ "text": r }),
            SessionEvent::ToolStart { id, name, input } => {
                serde_json::json!({ "id": id, "name": name, "input": input })
            }
            SessionEvent::ToolEnd {
                id,
                name,
                output,
                is_error,
            } => {
                serde_json::json!({ "id": id, "name": name, "output": output, "is_error": is_error })
            }
            SessionEvent::AgentSwitch(a) => serde_json::json!({ "agent": a }),
            SessionEvent::Compaction(s) => serde_json::json!({ "summary": s }),
            SessionEvent::Status(s) => serde_json::json!({ "status": s }),
            SessionEvent::Done => serde_json::json!({}),
            SessionEvent::Error(e) => serde_json::json!({ "error": e }),
            SessionEvent::SubagentStart {
                id,
                kind,
                prompt,
                child_session_id,
            } => {
                serde_json::json!({ "id": id, "kind": kind, "prompt": prompt, "child_session_id": child_session_id })
            }
            SessionEvent::SubagentEnd {
                id,
                ok,
                cancelled,
                summary,
            } => {
                serde_json::json!({ "id": id, "ok": ok, "cancelled": cancelled, "summary": summary })
            }
            SessionEvent::SubagentChild { id, ev } => {
                serde_json::json!({ "id": id, "event": ev })
            }
            SessionEvent::PlanHandoff(plan) => serde_json::json!({ "plan": plan }),
            SessionEvent::TranscriptReset(_) => serde_json::json!({}),
            SessionEvent::QueueConsumed { seq } => serde_json::json!({ "seq": seq }),
            SessionEvent::SteerConsumed { seq } => serde_json::json!({ "seq": seq }),
        }
    }

    /// Reconstruct a `SessionEvent` from an SSE event-name (`sse_kind`) and its
    /// payload (`sse_data`). This is the inverse of `sse_kind()` + `sse_data()`,
    /// letting a remote client (`opencode client`) rebuild the structured event
    /// stream from a server's `/events` SSE wire format.
    ///
    /// Returns `None` for an unrecognized `kind`. `TranscriptReset` carries no
    /// messages on the wire (its payload is `{}`), so it is returned as an empty
    /// marker — callers that need the rebuilt transcript must re-fetch
    /// `/messages`. `SubagentChild` deserializes its nested `event` as the enum
    /// (not the SSE form), matching how `sse_data` serializes it.
    pub fn from_sse(kind: &str, data: serde_json::Value) -> Option<Self> {
        Some(match kind {
            "text_delta" => SessionEvent::TextDelta(data.get("text")?.as_str()?.to_string()),
            "reasoning_delta" => {
                SessionEvent::ReasoningDelta(data.get("text")?.as_str()?.to_string())
            }
            "tool_start" => SessionEvent::ToolStart {
                id: data.get("id")?.as_str()?.to_string(),
                name: data.get("name")?.as_str()?.to_string(),
                input: data.get("input")?.clone(),
            },
            "tool_end" => SessionEvent::ToolEnd {
                id: data.get("id")?.as_str()?.to_string(),
                name: data.get("name")?.as_str()?.to_string(),
                output: data.get("output")?.as_str()?.to_string(),
                is_error: data.get("is_error")?.as_bool().unwrap_or(false),
            },
            "agent_switched" => SessionEvent::AgentSwitch(data.get("agent")?.as_str()?.to_string()),
            "compaction" => SessionEvent::Compaction(data.get("summary")?.as_str()?.to_string()),
            "status" => SessionEvent::Status(data.get("status")?.as_str()?.to_string()),
            "subagent_start" => SessionEvent::SubagentStart {
                id: data.get("id")?.as_str()?.to_string(),
                kind: data.get("kind")?.as_str()?.to_string(),
                prompt: data.get("prompt")?.as_str()?.to_string(),
                child_session_id: data.get("child_session_id")?.as_str()?.to_string(),
            },
            "subagent_end" => SessionEvent::SubagentEnd {
                id: data.get("id")?.as_str()?.to_string(),
                ok: data.get("ok")?.as_bool().unwrap_or(false),
                cancelled: data.get("cancelled")?.as_bool().unwrap_or(false),
                summary: data.get("summary")?.as_str()?.to_string(),
            },
            "subagent_child" => {
                let ev: SessionEvent = serde_json::from_value(data.get("event")?.clone()).ok()?;
                SessionEvent::SubagentChild {
                    id: data.get("id")?.as_str()?.to_string(),
                    ev: Box::new(ev),
                }
            }
            "plan_handoff" => SessionEvent::PlanHandoff(data.get("plan")?.as_str()?.to_string()),
            "transcript_reset" => {
                // Wire payload is `{}`; the rebuilt message list is intentionally
                // empty (see method doc). Callers re-fetch /messages if needed.
                SessionEvent::TranscriptReset(Vec::new())
            }
            "queue_consumed" => SessionEvent::QueueConsumed {
                seq: data.get("seq")?.as_i64().unwrap_or(0),
            },
            "steer_consumed" => SessionEvent::SteerConsumed {
                seq: data.get("seq")?.as_i64().unwrap_or(0),
            },
            "done" => SessionEvent::Done,
            "error" => SessionEvent::Error(data.get("error")?.as_str()?.to_string()),
            _ => return None,
        })
    }

    /// Coarse [`EventKind`] for backward-compatible DB `type` column.
    pub fn coarse_kind(&self) -> EventKind {
        match self {
            SessionEvent::TextDelta(_) => EventKind::TextDelta,
            SessionEvent::ReasoningDelta(_) => EventKind::TextDelta,
            SessionEvent::ToolStart { .. } => EventKind::ToolStart,
            SessionEvent::ToolEnd { .. } => EventKind::ToolEnd,
            SessionEvent::AgentSwitch(_) => EventKind::AgentSwitched,
            SessionEvent::Compaction(_) => EventKind::Compaction,
            SessionEvent::Status(_) => EventKind::Step,
            SessionEvent::Done => EventKind::Done,
            SessionEvent::Error(_) => EventKind::Error,
            SessionEvent::SubagentStart { .. }
            | SessionEvent::SubagentEnd { .. }
            | SessionEvent::SubagentChild { .. }
            | SessionEvent::PlanHandoff(_)
            | SessionEvent::QueueConsumed { .. }
            | SessionEvent::SteerConsumed { .. } => EventKind::Step,
            SessionEvent::TranscriptReset(_) => EventKind::Compaction,
        }
    }
}


pub(super) const MAX_OUTPUT: usize = 4096;
pub(super) const DOOM_THRESHOLD: usize = 3;

/// Shared event sink for concurrent tool dispatch. Wraps the borrowed `FnMut`
/// closure in a `Mutex` so multiple in-flight tool/subagent futures can emit
/// events safely (emissions serialize; each is a fast push). The lifetime is
/// bound to the caller's closure — no `'static` requirement, so test closures
/// that borrow local state keep working unmodified.
pub(super) type Sink<'a> = Arc<Mutex<&'a mut (dyn FnMut(SessionEvent) + Send)>>;

#[cfg(test)]
mod from_sse_tests {
    use super::*;

    /// `from_sse` is the exact inverse of `sse_kind()` + `sse_data()` for every
    /// variant EXCEPT `TranscriptReset`, whose payload is `{}` on the wire
    /// (the rebuilt message list cannot be carried over SSE and must be
    /// re-fetched). Pin both the roundtrip and that documented lossiness.
    #[test]
    fn from_sse_roundtrips_all_variants() {
        let cases: Vec<SessionEvent> = vec![
            SessionEvent::TextDelta("hi".into()),
            SessionEvent::ReasoningDelta("think".into()),
            SessionEvent::ToolStart {
                id: "t1".into(),
                name: "bash".into(),
                input: serde_json::json!({"command": "ls"}),
            },
            SessionEvent::ToolEnd {
                id: "t1".into(),
                name: "bash".into(),
                output: "done".into(),
                is_error: false,
            },
            SessionEvent::ToolEnd {
                id: "t2".into(),
                name: "bash".into(),
                output: "boom".into(),
                is_error: true,
            },
            SessionEvent::AgentSwitch("plan".into()),
            SessionEvent::Compaction("summary".into()),
            SessionEvent::Status("running".into()),
            SessionEvent::SubagentStart {
                id: "s1".into(),
                kind: "explore".into(),
                prompt: "find x".into(),
                child_session_id: "child-1".into(),
            },
            SessionEvent::SubagentEnd {
                id: "s1".into(),
                ok: true,
                cancelled: false,
                summary: "found".into(),
            },
            SessionEvent::SubagentChild {
                id: "s1".into(),
                ev: Box::new(SessionEvent::TextDelta("child text".into())),
            },
            SessionEvent::PlanHandoff("# plan".into()),
            SessionEvent::TranscriptReset(vec![Message::assistant("m1")]),
            SessionEvent::QueueConsumed { seq: 7 },
            SessionEvent::SteerConsumed { seq: 9 },
            SessionEvent::Done,
            SessionEvent::Error("kaboom".into()),
        ];
        let mut kinds: Vec<&str> = cases.iter().map(|e| e.sse_kind()).collect();
        kinds.sort();
        kinds.dedup();
        assert_eq!(
            kinds.len(),
            16,
            "expected all 16 unique kinds, got {kinds:?}"
        );

        for ev in &cases {
            let kind = ev.sse_kind();
            let data = ev.sse_data();
            let back = SessionEvent::from_sse(kind, data.clone())
                .unwrap_or_else(|| panic!("from_sse returned None for kind={kind} data={data}"));
            if matches!(ev, SessionEvent::TranscriptReset(_)) {
                // documented lossiness: no messages on the wire
                assert!(matches!(back, SessionEvent::TranscriptReset(ref v) if v.is_empty()));
            } else {
                assert_eq!(
                    serde_json::to_string(&back).unwrap(),
                    serde_json::to_string(ev).unwrap(),
                    "roundtrip mismatch for kind={kind}"
                );
            }
        }
    }

    #[test]
    fn from_sse_unknown_kind_is_none() {
        assert!(SessionEvent::from_sse("no_such_kind", serde_json::json!({})).is_none());
    }

    #[test]
    fn from_sse_missing_field_is_none() {
        // tool_start without the required `name` field
        assert!(SessionEvent::from_sse("tool_start", serde_json::json!({"id":"x"})).is_none());
    }
}

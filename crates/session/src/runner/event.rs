use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use opencoder_core::Message;

use crate::autopilot::state::ApPhase;
use opencoder_store::EventKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionEvent {
    /// One provider/model round started. This is display-only lifecycle data:
    /// it never enters [`Message`] content or the next model request context.
    LlmRoundStart {
        started_at_ms: i64,
    },
    /// The current provider/model round finished, including every tool call
    /// requested by its assistant message.
    LlmRoundEnd,
    /// Real token usage of one completed provider round, emitted right after
    /// the assistant message (with its `usage`) is persisted. Carries the
    /// provider-reported `total_tokens` (input+output, incl. cache) so display
    /// surfaces can accumulate a session-lifetime cost without re-reading
    /// messages. The split `input_tokens`/`output_tokens` let the TUI show the
    /// provider-truth context size of the latest round. Parent views fold
    /// subagent rounds into their lifetime cost too (the runner forwards each
    /// child round as `SubagentChild(LlmUsage)`). Emitted only when the
    /// provider returned usage; rounds without usage simply contribute
    /// nothing. Old persisted payloads predate the split fields and
    /// deserialize them to `0` (`#[serde(default)]`).
    LlmUsage {
        total_tokens: u64,
        #[serde(default)]
        input_tokens: u64,
        #[serde(default)]
        output_tokens: u64,
    },
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
        /// Image attachments returned by a tool (data URIs or URLs), rendered
        /// inline in the TUI transcript alongside the text output. Defaults to
        /// empty when absent (backward-compatible with old persisted events).
        #[serde(default)]
        images: Vec<String>,
    },
    AgentSwitch(String),
    /// The active model was switched at runtime (e.g. via the `/model` menu
    /// or the web `POST /sessions/:id/model` endpoint). Carries the new
    /// `"provider/model_id"` string so display surfaces and resume stay in
    /// sync with the on-disk config.
    ModelSwitch(String),
    Compaction(String),
    CompactionDelta(String),
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
    /// A queued follow-up was consumed (drained) at an idle boundary. Carries
    /// the consumed input's row seq so the TUI can drop it from its pending
    /// mirror instead of leaving a stale `[queued]` row until `Done`.
    QueueConsumed {
        seq: i64,
        /// The consumed prompt text so display surfaces can echo it at the
        /// exact activation instant — without this, stateless clients (web /
        /// CLI) cannot show the text until the turn finishes and `/messages`
        /// is re-fetched. Defaults to empty for old persisted events.
        #[serde(default)]
        text: String,
    },
    /// A steered input was consumed (promoted) at a turn boundary. Carries
    /// the consumed input's row seq so the TUI can drop it from its pending
    /// mirror instead of leaving a stale `steer` row until `Done`.
    SteerConsumed {
        seq: i64,
        /// The promoted steer prompt text, same rationale as `QueueConsumed`.
        #[serde(default)]
        text: String,
    },
    /// Autopilot loop progress: which phase is starting and the
    /// 0-based iteration index. Emitted by `autopilot::drive`.
    AutoPilot {
        phase: ApPhase,
        iteration: u32,
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
            SessionEvent::LlmRoundStart { .. } => "llm_round_start",
            SessionEvent::LlmRoundEnd => "llm_round_end",
            SessionEvent::LlmUsage { .. } => "llm_usage",
            SessionEvent::TextDelta(_) => "text_delta",
            SessionEvent::ReasoningDelta(_) => "reasoning_delta",
            SessionEvent::ToolStart { .. } => "tool_start",
            SessionEvent::ToolEnd { .. } => "tool_end",
            SessionEvent::AgentSwitch(_) => "agent_switched",
            SessionEvent::ModelSwitch(_) => "model_switched",
            SessionEvent::Compaction(_) => "compaction",
            SessionEvent::CompactionDelta(_) => "compaction_delta",
            SessionEvent::Status(_) => "status",
            SessionEvent::Done => "done",
            SessionEvent::Error(_) => "error",
            SessionEvent::SubagentStart { .. } => "subagent_start",
            SessionEvent::SubagentEnd { .. } => "subagent_end",
            SessionEvent::SubagentChild { .. } => "subagent_child",
            SessionEvent::AutoPilot { .. } => "autopilot",
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
            SessionEvent::LlmRoundStart { started_at_ms } => {
                serde_json::json!({ "started_at_ms": started_at_ms })
            }
            SessionEvent::LlmRoundEnd => serde_json::json!({}),
            SessionEvent::LlmUsage {
                total_tokens,
                input_tokens,
                output_tokens,
            } => serde_json::json!({
                "total_tokens": total_tokens,
                "input_tokens": input_tokens,
                "output_tokens": output_tokens
            }),
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
                images,
            } => {
                serde_json::json!({ "id": id, "name": name, "output": output, "is_error": is_error, "images": images })
            }
            SessionEvent::AgentSwitch(a) => serde_json::json!({ "agent": a }),
            SessionEvent::ModelSwitch(m) => serde_json::json!({ "model": m }),
            SessionEvent::Compaction(s) => serde_json::json!({ "summary": s }),
            SessionEvent::CompactionDelta(t) => serde_json::json!({ "text": t }),
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
            SessionEvent::AutoPilot { phase, iteration } => {
                serde_json::json!({ "phase": phase, "iteration": iteration })
            }
            SessionEvent::TranscriptReset(_) => serde_json::json!({}),
            SessionEvent::QueueConsumed { seq, text } => {
                serde_json::json!({ "seq": seq, "text": text })
            }
            SessionEvent::SteerConsumed { seq, text } => {
                serde_json::json!({ "seq": seq, "text": text })
            }
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
            "llm_round_start" => SessionEvent::LlmRoundStart {
                started_at_ms: data.get("started_at_ms")?.as_i64()?,
            },
            "llm_round_end" => SessionEvent::LlmRoundEnd,
            "llm_usage" => SessionEvent::LlmUsage {
                total_tokens: data.get("total_tokens")?.as_u64()?,
                // Old payloads carry only total_tokens; the split defaults to 0.
                input_tokens: data
                    .get("input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or_default(),
                output_tokens: data
                    .get("output_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or_default(),
            },
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
                images: data
                    .get("images")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
            },
            "agent_switched" => SessionEvent::AgentSwitch(data.get("agent")?.as_str()?.to_string()),
            "model_switched" => SessionEvent::ModelSwitch(data.get("model")?.as_str()?.to_string()),
            "compaction" => SessionEvent::Compaction(data.get("summary")?.as_str()?.to_string()),
            "compaction_delta" => {
                SessionEvent::CompactionDelta(data.get("text")?.as_str()?.to_string())
            }
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
            "transcript_reset" => {
                // Wire payload is `{}`; the rebuilt message list is intentionally
                // empty (see method doc). Callers re-fetch /messages if needed.
                SessionEvent::TranscriptReset(Vec::new())
            }
            "queue_consumed" => SessionEvent::QueueConsumed {
                seq: data.get("seq")?.as_i64()?,
                text: data
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            },
            "steer_consumed" => SessionEvent::SteerConsumed {
                seq: data.get("seq")?.as_i64()?,
                text: data
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            },
            "autopilot" => {
                let iteration = data.get("iteration")?.as_u64()?.min(u32::MAX as u64) as u32;
                let phase = serde_json::from_value(data.get("phase")?.clone()).ok()?;
                SessionEvent::AutoPilot { phase, iteration }
            }
            "done" => SessionEvent::Done,
            "error" => SessionEvent::Error(data.get("error")?.as_str()?.to_string()),
            _ => return None,
        })
    }

    /// Coarse [`EventKind`] for backward-compatible DB `type` column.
    pub fn coarse_kind(&self) -> EventKind {
        match self {
            SessionEvent::LlmRoundStart { .. }
            | SessionEvent::LlmRoundEnd
            | SessionEvent::LlmUsage { .. } => EventKind::Step,
            SessionEvent::TextDelta(_) => EventKind::TextDelta,
            SessionEvent::ReasoningDelta(_) => EventKind::TextDelta,
            SessionEvent::ToolStart { .. } => EventKind::ToolStart,
            SessionEvent::ToolEnd { .. } => EventKind::ToolEnd,
            SessionEvent::AgentSwitch(_) => EventKind::AgentSwitched,
            SessionEvent::ModelSwitch(_) => EventKind::ModelSwitched,
            SessionEvent::Compaction(_) => EventKind::Compaction,
            SessionEvent::CompactionDelta(_) => EventKind::Compaction,
            SessionEvent::Status(_) => EventKind::Step,
            SessionEvent::Done => EventKind::Done,
            SessionEvent::Error(_) => EventKind::Error,
            SessionEvent::SubagentStart { .. }
            | SessionEvent::SubagentEnd { .. }
            | SessionEvent::SubagentChild { .. }
            | SessionEvent::AutoPilot { .. }
            | SessionEvent::QueueConsumed { .. }
            | SessionEvent::SteerConsumed { .. } => EventKind::Step,
            SessionEvent::TranscriptReset(_) => EventKind::Compaction,
        }
    }
}

pub(super) const MAX_OUTPUT: usize = 4096;
pub(super) const DOOM_THRESHOLD: usize = 20;

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
            SessionEvent::LlmRoundStart {
                started_at_ms: 1234,
            },
            SessionEvent::LlmRoundEnd,
            SessionEvent::LlmUsage {
                total_tokens: 123_456,
                input_tokens: 100_000,
                output_tokens: 23_456,
            },
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
                images: Vec::new(),
            },
            SessionEvent::ToolEnd {
                id: "t2".into(),
                name: "bash".into(),
                output: "boom".into(),
                is_error: true,
                images: Vec::new(),
            },
            SessionEvent::AgentSwitch("sandbox".into()),
            SessionEvent::ModelSwitch("openai/gpt-4o".into()),
            SessionEvent::Compaction("summary".into()),
            SessionEvent::CompactionDelta("cdelta".into()),
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
            SessionEvent::TranscriptReset(vec![Message::assistant("m1")]),
            SessionEvent::QueueConsumed {
                seq: 7,
                text: "q".into(),
            },
            SessionEvent::SteerConsumed {
                seq: 9,
                text: "s".into(),
            },
            SessionEvent::AutoPilot {
                phase: ApPhase::Plan,
                iteration: 0,
            },
            SessionEvent::Done,
            SessionEvent::Error("kaboom".into()),
        ];
        let mut kinds: Vec<&str> = cases.iter().map(|e| e.sse_kind()).collect();
        kinds.sort();
        kinds.dedup();
        assert_eq!(
            kinds.len(),
            21,
            "expected all 21 unique kinds, got {kinds:?}"
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

    /// Backward compatibility: llm_usage payloads persisted before the
    /// input/output split must still deserialize (split fields default to 0)
    /// — both on the SSE wire form and the direct enum form used by the store.
    #[test]
    fn llm_usage_old_payload_defaults_split_fields_to_zero() {
        let ev = SessionEvent::from_sse("llm_usage", serde_json::json!({ "total_tokens": 42 }))
            .expect("old llm_usage payload must parse");
        match ev {
            SessionEvent::LlmUsage {
                total_tokens,
                input_tokens,
                output_tokens,
            } => {
                assert_eq!(total_tokens, 42);
                assert_eq!(input_tokens, 0);
                assert_eq!(output_tokens, 0);
            }
            other => panic!("expected LlmUsage, got {other:?}"),
        }
        let stored: SessionEvent =
            serde_json::from_str(r#"{"LlmUsage":{"total_tokens":42}}"#).unwrap();
        assert!(matches!(
            stored,
            SessionEvent::LlmUsage {
                total_tokens: 42,
                input_tokens: 0,
                output_tokens: 0,
            }
        ));
    }

    #[test]
    fn from_sse_missing_field_is_none() {
        // tool_start without the required `name` field
        assert!(SessionEvent::from_sse("tool_start", serde_json::json!({"id":"x"})).is_none());
    }

    #[test]
    fn queue_consumed_carries_text_through_sse() {
        let ev = SessionEvent::QueueConsumed {
            seq: 5,
            text: "hello queued".into(),
        };
        let kind = ev.sse_kind();
        assert_eq!(kind, "queue_consumed");
        let data = ev.sse_data();
        assert_eq!(data["text"], "hello queued");
        assert_eq!(data["seq"], 5);
        let back = SessionEvent::from_sse(kind, data).expect("roundtrip");
        match back {
            SessionEvent::QueueConsumed { seq, text } => {
                assert_eq!(seq, 5);
                assert_eq!(text, "hello queued");
            }
            other => panic!("expected QueueConsumed, got {other:?}"),
        }
    }

    #[test]
    fn steer_consumed_carries_text_through_sse() {
        let ev = SessionEvent::SteerConsumed {
            seq: 9,
            text: "steered away".into(),
        };
        let kind = ev.sse_kind();
        assert_eq!(kind, "steer_consumed");
        let data = ev.sse_data();
        assert_eq!(data["text"], "steered away");
        assert_eq!(data["seq"], 9);
        let back = SessionEvent::from_sse(kind, data).expect("roundtrip");
        match back {
            SessionEvent::SteerConsumed { seq, text } => {
                assert_eq!(seq, 9);
                assert_eq!(text, "steered away");
            }
            other => panic!("expected SteerConsumed, got {other:?}"),
        }
    }

    #[test]
    fn queue_consumed_without_text_field_is_backward_compatible() {
        // Old persisted events predate the `text` field. A queue_consumed SSE
        // payload without the key must still deserialize (defaults to empty).
        let data = serde_json::json!({ "seq": 11 });
        let ev = SessionEvent::from_sse("queue_consumed", data).expect("old event");
        match ev {
            SessionEvent::QueueConsumed { seq, text } => {
                assert_eq!(seq, 11);
                assert!(text.is_empty(), "missing text must default to empty");
            }
            other => panic!("expected QueueConsumed, got {other:?}"),
        }
    }

    #[test]
    fn steer_consumed_without_text_field_is_backward_compatible() {
        let data = serde_json::json!({ "seq": 13 });
        let ev = SessionEvent::from_sse("steer_consumed", data).expect("old event");
        match ev {
            SessionEvent::SteerConsumed { seq, text } => {
                assert_eq!(seq, 13);
                assert!(text.is_empty());
            }
            other => panic!("expected SteerConsumed, got {other:?}"),
        }
    }

    #[test]
    fn tool_end_images_roundtrip_through_sse() {
        let ev = SessionEvent::ToolEnd {
            id: "img-1".into(),
            name: "view_image".into(),
            output: "Loaded image: cat.png".into(),
            is_error: false,
            images: vec![
                "data:image/png;base64,iVBORw0KGgo=".into(),
                "https://example.com/photo.jpg".into(),
            ],
        };
        let kind = ev.sse_kind();
        let data = ev.sse_data();
        let back = SessionEvent::from_sse(kind, data).expect("roundtrip");
        match back {
            SessionEvent::ToolEnd { images, .. } => {
                assert_eq!(images.len(), 2, "images must survive roundtrip");
                assert_eq!(images[0], "data:image/png;base64,iVBORw0KGgo=");
                assert_eq!(images[1], "https://example.com/photo.jpg");
            }
            other => panic!("expected ToolEnd, got {other:?}"),
        }
    }

    #[test]
    fn tool_end_without_images_field_is_backward_compatible() {
        // Old persisted events predate the `images` field. A tool_end SSE
        // payload without the key must still deserialize (defaults to empty).
        let data = serde_json::json!({
            "id": "old",
            "name": "bash",
            "output": "done",
            "is_error": false,
        });
        let ev = SessionEvent::from_sse("tool_end", data).expect("old event");
        match ev {
            SessionEvent::ToolEnd { images, .. } => {
                assert!(
                    images.is_empty(),
                    "missing images field must default to empty"
                );
            }
            other => panic!("expected ToolEnd, got {other:?}"),
        }
    }

    /// P2-6: `from_sse` must saturate `iteration` to u32::MAX when the JSON
    /// value exceeds u32's range (e.g. u64::MAX). The old `as u32` cast
    /// silently wrapped to a small number, producing a wrong iteration index.
    #[test]
    fn from_sse_autopilot_large_iteration_saturates() {
        let data = serde_json::json!({
            "phase": "act",
            "iteration": u64::MAX,
        });
        let ev = SessionEvent::from_sse("autopilot", data).expect("must parse");
        match ev {
            SessionEvent::AutoPilot { iteration, .. } => {
                assert_eq!(
                    iteration,
                    u32::MAX,
                    "iteration must saturate to u32::MAX, not wrap"
                );
            }
            other => panic!("expected AutoPilot, got {other:?}"),
        }
    }
}

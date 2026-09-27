//! Session-lifetime LLM token totals derived from the persisted event log.
//!
//! The live footer accumulates every `llm_usage` frame the stream delivers —
//! the parent's own rounds plus child rounds forwarded as wrapped
//! `subagent_child(llm_usage)` frames (task-tool subagents and local-memory
//! maintenance). Child spend never lands on a parent message row, so a
//! reloaded console cannot recover it from `messages` alone: the event log
//! is the only source that matches what the live footer accumulated. The
//! snapshot endpoint exposes the aggregate as `usage`; the SPA prefers it
//! and falls back to per-message sums for pre-sink legacy sessions.

use std::sync::Arc;

use opencoder_session::SessionEvent;
use opencoder_store::{SessionEventRecord, Store};
use serde_json::{json, Value};

/// Aggregated lifetime usage of one session. `None` (via the fallible
/// collectors) when the log holds no usage-bearing rows at all.
#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub(crate) struct UsageTotals {
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl UsageTotals {
    fn add_round(&mut self, total: u64, input: u64, output: u64) {
        // A round without provider usage contributes nothing (the runner
        // emits no frame for it); a frame with a zero total is a no-op too.
        if total == 0 && input == 0 && output == 0 {
            return;
        }
        self.total_tokens = self.total_tokens.saturating_add(total);
        // Pre-split legacy rows deserialize the split fields to 0: the Σ
        // still grows, the split stays honest (no derived fake numbers).
        self.input_tokens = self.input_tokens.saturating_add(input);
        self.output_tokens = self.output_tokens.saturating_add(output);
    }

    fn seen(&self) -> bool {
        self.total_tokens > 0 || self.input_tokens > 0 || self.output_tokens > 0
    }
}

/// Fold the usage-bearing subset of a session's event records. Pure — the
/// store query lives in [`session_usage`].
pub(crate) fn sum_usage_records(records: &[SessionEventRecord]) -> Option<UsageTotals> {
    let mut totals = UsageTotals::default();
    for rec in records {
        match SessionEvent::from_stored(rec) {
            Some(SessionEvent::LlmUsage {
                total_tokens,
                input_tokens,
                output_tokens,
            }) => totals.add_round(total_tokens, input_tokens, output_tokens),
            Some(SessionEvent::SubagentChild { ev, .. }) => {
                if let SessionEvent::LlmUsage {
                    total_tokens,
                    input_tokens,
                    output_tokens,
                } = *ev
                {
                    totals.add_round(total_tokens, input_tokens, output_tokens);
                }
            }
            _ => {}
        }
    }
    totals.seen().then_some(totals)
}

/// Lifetime usage of one session from its persisted `llm_usage` +
/// `subagent_child` rows. `None` on a store error or an empty usage log —
/// callers fall back to per-message sums, never fail the snapshot.
pub(crate) async fn session_usage(store: &Arc<dyn Store>, session_id: &str) -> Option<UsageTotals> {
    let records = store
        .events_of_kinds(session_id, &["llm_usage", "subagent_child"])
        .await
        .ok()?;
    sum_usage_records(&records)
}

/// Wire form for the snapshot JSON: `null` when absent so clients keep their
/// message-sum fallback instead of rendering an all-zero footer.
pub(crate) fn usage_json(usage: Option<UsageTotals>) -> Value {
    match usage {
        Some(u) => json!({
            "total_tokens": u.total_tokens,
            "input_tokens": u.input_tokens,
            "output_tokens": u.output_tokens,
        }),
        None => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(sse_kind: Option<&str>, payload: Value) -> SessionEventRecord {
        SessionEventRecord {
            session_id: "s".into(),
            kind: opencoder_store::EventKind::Step,
            payload,
            ts: 1,
            seq: None,
            sse_kind: sse_kind.map(str::to_string),
        }
    }

    fn usage_ev(total: u64, input: u64, output: u64) -> SessionEvent {
        SessionEvent::LlmUsage {
            total_tokens: total,
            input_tokens: input,
            output_tokens: output,
        }
    }

    /// Parent rounds + a wrapped memory round + noise rows → one total.
    #[test]
    fn sums_bare_and_wrapped_usage_frames() {
        let wrapped = SessionEvent::SubagentChild {
            id: "memory-1".into(),
            ev: Box::new(usage_ev(1234, 1000, 234)),
        };
        let noise = usage_ev(0, 0, 0);
        let records = vec![
            row(Some("llm_usage"), usage_ev(500, 400, 100).sse_data()),
            row(Some("text_delta"), serde_json::json!({"text": "x"})),
            row(Some("subagent_child"), wrapped.sse_data()),
            row(Some("llm_usage"), noise.sse_data()),
        ];
        assert_eq!(
            sum_usage_records(&records),
            Some(UsageTotals {
                total_tokens: 1734,
                input_tokens: 1400,
                output_tokens: 334,
            })
        );
    }

    /// Legacy enum-form rows (the `resume.rs` writer) parse through the same
    /// collector, and a usage-free log yields `None` (client fallback).
    #[test]
    fn enum_form_rows_count_and_empty_logs_yield_none() {
        let enum_row = row(
            Some("llm_usage"),
            serde_json::to_value(usage_ev(42, 40, 2)).unwrap(),
        );
        assert_eq!(
            sum_usage_records(&[enum_row]),
            Some(UsageTotals {
                total_tokens: 42,
                input_tokens: 40,
                output_tokens: 2,
            })
        );
        assert_eq!(sum_usage_records(&[]), None);
        assert_eq!(
            sum_usage_records(&[row(Some("text_delta"), serde_json::json!({"text": "x"}))]),
            None
        );
        assert_eq!(usage_json(None), Value::Null);
    }
}

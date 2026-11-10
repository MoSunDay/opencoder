//! Last-resort wire sanitizer for the OpenAI tool-call pairing contract.
//!
//! `lower_messages` produces wire-legal output for well-formed sessions: every
//! assistant `tool_calls` id is answered by exactly one `role:"tool"` message
//! that immediately follows it. Transcripts corrupted at hard-cancel or
//! compaction boundaries (an orphaned tool result, an assistant whose calls
//! lost their answers to a truncation, a duplicated result replay) violate
//! that contract, and strict OpenAI-compatible backends reject such bodies
//! with HTTP 400 before the model sees anything.
//!
//! This module is the safety net below `lower_messages`: a pure, single-pass
//! state machine over already-lowered wire messages that repairs the pairing
//! so `ChatRequest::to_body` stays wire-legal. It never reorders or buffers —
//! it only drops or strips in place. Dropping (rather than reordering) is
//! deliberate: the dropped payloads are duplicates or error placeholders, and
//! a legal continuation matters more than their content.

use serde_json::Value;

/// Counters describing every repair one `sanitize_tool_messages` pass made.
/// Plain data only — all behaviour lives in free functions below.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SanitizeReport {
    /// `role:"tool"` messages removed (orphaned, unknown/duplicate id, or
    /// arriving after the group had already closed).
    pub(crate) dropped_tool_messages: usize,
    /// Assistant `tool_calls` entries removed because no `role:"tool"`
    /// message answered them.
    pub(crate) stripped_tool_calls: usize,
    /// Whole assistant messages removed: all of their calls went unanswered
    /// and they carried no usable text/reasoning payload.
    pub(crate) dropped_assistant_messages: usize,
}

impl SanitizeReport {
    /// True when the input needed no repair at all.
    pub(crate) fn is_clean(&self) -> bool {
        self.dropped_tool_messages == 0
            && self.stripped_tool_calls == 0
            && self.dropped_assistant_messages == 0
    }
}

/// One call inside the currently open group. `id: None` marks a `tool_calls`
/// entry whose `id` is missing or not a string — it can never be answered, so
/// it always counts as unanswered and is stripped at close.
struct GroupCall {
    id: Option<String>,
    answered: bool,
}

/// The group opened by the last assistant message carrying `tool_calls`:
/// where that assistant sits in the output vec, plus one answer flag per call.
/// A group is closed by any non-tool message, a following assistant message,
/// or the end of input — before the next one opens, so `assistant_idx` stays
/// valid for in-place repair.
struct OpenGroup {
    assistant_idx: usize,
    calls: Vec<GroupCall>,
}

/// Sanitize already-lowered wire messages in one pass, dropping or stripping
/// in place (never reordering) so every assistant `tool_calls` id is answered
/// by exactly one immediately-following `role:"tool"` message. Returns the
/// repaired stream plus a report of what was removed.
pub(crate) fn sanitize_tool_messages(messages: Vec<Value>) -> (Vec<Value>, SanitizeReport) {
    let mut out: Vec<Value> = Vec::with_capacity(messages.len());
    let mut report = SanitizeReport::default();
    let mut group: Option<OpenGroup> = None;

    for msg in messages {
        match msg.get("role").and_then(Value::as_str) {
            Some("tool") => match accepted_tool_id(group.as_ref(), &msg) {
                Some(id) => {
                    mark_answered(&mut group, &id);
                    out.push(msg);
                }
                None => report.dropped_tool_messages += 1,
            },
            Some("assistant") => {
                close_group(group.take(), &mut out, &mut report);
                group = open_group(&msg, out.len());
                out.push(msg);
            }
            _ => {
                close_group(group.take(), &mut out, &mut report);
                out.push(msg);
            }
        }
    }
    close_group(group, &mut out, &mut report);
    (out, report)
}

/// Start a group at an assistant message that carries a `tool_calls` array;
/// assistants without one leave the group closed (`None`).
fn open_group(msg: &Value, assistant_idx: usize) -> Option<OpenGroup> {
    let calls = msg.get("tool_calls")?.as_array()?;
    Some(OpenGroup {
        assistant_idx,
        calls: calls
            .iter()
            .map(|call| GroupCall {
                id: call.get("id").and_then(Value::as_str).map(str::to_string),
                answered: false,
            })
            .collect(),
    })
}

/// Decide whether a `role:"tool"` message may be emitted: the group must be
/// open, its `tool_call_id` a non-empty string that belongs to the group's
/// calls, and not answered yet. Returns the answered id on acceptance.
fn accepted_tool_id(group: Option<&OpenGroup>, msg: &Value) -> Option<String> {
    let group = group?;
    let id = msg.get("tool_call_id").and_then(Value::as_str)?;
    if id.is_empty() {
        return None;
    }
    group
        .calls
        .iter()
        .find(|call| call.id.as_deref() == Some(id) && !call.answered)
        .map(|call| {
            call.id
                .clone()
                .expect("matching calls always carry a string id")
        })
}

/// Flag every group call sharing the answered id (duplicate ids in one
/// assistant share the answer; duplicates after it get dropped instead).
fn mark_answered(group: &mut Option<OpenGroup>, id: &str) {
    if let Some(open) = group {
        for call in &mut open.calls {
            if call.id.as_deref() == Some(id) {
                call.answered = true;
            }
        }
    }
}

/// Close the open group, if any. When every call was answered the assistant is
/// left byte-identical; otherwise unanswered calls are stripped from its
/// `tool_calls` array, and an assistant left with neither calls nor usable
/// `content`/`reasoning_content` payload is removed entirely.
fn close_group(group: Option<OpenGroup>, out: &mut Vec<Value>, report: &mut SanitizeReport) {
    let Some(group) = group else {
        return;
    };
    let answered: Vec<&str> = group
        .calls
        .iter()
        .filter(|call| call.answered)
        .filter_map(|call| call.id.as_deref())
        .collect();
    if answered.len() == group.calls.len() {
        return;
    }
    report.stripped_tool_calls += group.calls.len() - answered.len();
    let retained = retained_calls(out, group.assistant_idx, &answered);
    if !retained.is_empty() {
        if let Some(assistant) = out
            .get_mut(group.assistant_idx)
            .and_then(Value::as_object_mut)
        {
            assistant.insert("tool_calls".to_string(), Value::Array(retained));
        }
        return;
    }
    if out.get(group.assistant_idx).is_some_and(has_usable_payload) {
        if let Some(assistant) = out
            .get_mut(group.assistant_idx)
            .and_then(Value::as_object_mut)
        {
            // An empty `tool_calls` array is spec-invalid; strip the key.
            assistant.remove("tool_calls");
        }
    } else {
        out.remove(group.assistant_idx);
        report.dropped_assistant_messages += 1;
    }
}

/// The `tool_calls` entries whose id is an answered id, in original order.
fn retained_calls(out: &[Value], assistant_idx: usize, answered: &[&str]) -> Vec<Value> {
    out.get(assistant_idx)
        .and_then(|assistant| assistant.get("tool_calls"))
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .filter(|call| {
                    call.get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| answered.contains(&id))
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// True when the assistant message keeps a payload worth sending after its
/// calls were stripped: non-empty `content` (absent/null/empty don't count)
/// or non-empty `reasoning_content`.
fn has_usable_payload(msg: &Value) -> bool {
    let usable_content = match msg.get("content") {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    };
    let usable_reasoning = msg
        .get("reasoning_content")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty());
    usable_content || usable_reasoning
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(id: &str) -> Value {
        json!({ "id": id, "type": "function", "function": { "name": "f", "arguments": "{}" } })
    }

    fn tool_msg(id: Option<&str>, content: &str) -> Value {
        let id = match id {
            Some(id) => json!(id),
            None => Value::Null,
        };
        json!({ "role": "tool", "tool_call_id": id, "content": content })
    }

    #[test]
    fn legal_transcript_passes_through_identical() {
        let msgs = vec![
            json!({ "role": "user", "content": "hi" }),
            json!({ "role": "assistant", "content": null, "tool_calls": [call("a"), call("b")] }),
            tool_msg(Some("a"), "ra"),
            tool_msg(Some("b"), "rb"),
            json!({ "role": "user", "content": "again" }),
        ];
        let (out, report) = sanitize_tool_messages(msgs.clone());
        assert_eq!(out, msgs, "legal input must be untouched (Value equality)");
        assert_eq!(
            serde_json::to_string(&out).unwrap(),
            serde_json::to_string(&msgs).unwrap(),
            "legal input must be byte-identical"
        );
        assert!(report.is_clean());
    }

    #[test]
    fn tool_message_with_missing_or_non_string_id_is_dropped() {
        let msgs = vec![
            json!({ "role": "assistant", "content": null, "tool_calls": [call("a")] }),
            json!({ "role": "tool", "content": "no id" }),
            json!({ "role": "assistant", "content": null, "tool_calls": [call("b")] }),
            json!({ "role": "tool", "tool_call_id": 42, "content": "numeric id" }),
        ];
        let (out, report) = sanitize_tool_messages(msgs);
        assert_eq!(
            report.dropped_tool_messages, 2,
            "missing and non-string ids must both count as dropped"
        );
        assert_eq!(report.stripped_tool_calls, 2);
        assert_eq!(report.dropped_assistant_messages, 2);
        assert!(out.is_empty(), "no answerable payload remains");
    }

    #[test]
    fn report_counters_on_mixed_dirty_input() {
        let msgs = vec![
            tool_msg(Some("x"), "orphan before any assistant"),
            json!({ "role": "assistant", "content": "text", "tool_calls": [call("a"), call("b")] }),
            tool_msg(Some("a"), "ra"),
            json!({ "role": "user", "content": "interleaved" }),
            tool_msg(Some("b"), "late"),
        ];
        let (out, report) = sanitize_tool_messages(msgs);
        assert_eq!(report.dropped_tool_messages, 2);
        assert_eq!(report.stripped_tool_calls, 1);
        assert_eq!(report.dropped_assistant_messages, 0);
        assert!(!report.is_clean());
        let assistant = out
            .iter()
            .find(|m| m["role"] == "assistant")
            .expect("text-bearing assistant must be kept");
        let ids: Vec<&str> = assistant["tool_calls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["a"], "only the answered call survives");
    }

    #[test]
    fn sanitize_is_idempotent_on_dirty_input() {
        let msgs = vec![
            tool_msg(Some("x"), "orphan"),
            json!({ "role": "assistant", "content": null, "tool_calls": [call("a"), call("b")] }),
            tool_msg(Some("a"), "ra"),
            tool_msg(Some("a"), "duplicate"),
            json!({ "role": "user", "content": "next" }),
        ];
        let (once, first) = sanitize_tool_messages(msgs);
        assert!(!first.is_clean(), "first pass must repair something");
        let (_, second) = sanitize_tool_messages(once);
        assert!(second.is_clean(), "second pass must find nothing to do");
    }

    #[test]
    fn duplicate_tool_result_keeps_only_first() {
        let msgs = vec![
            json!({ "role": "assistant", "content": null, "tool_calls": [call("a")] }),
            tool_msg(Some("a"), "first"),
            tool_msg(Some("a"), "second"),
        ];
        let (out, report) = sanitize_tool_messages(msgs);
        assert_eq!(report.dropped_tool_messages, 1);
        assert_eq!(
            out.iter().filter(|m| m["role"] == "tool").count(),
            1,
            "only the first answer may remain"
        );
        assert_eq!(out[1]["content"], "first");
        assert!(!report.is_clean(), "one duplicate was dropped");
    }

    #[test]
    fn tool_result_before_any_assistant_is_dropped() {
        let msgs = vec![
            tool_msg(Some("a"), "too early"),
            json!({ "role": "user", "content": "hi" }),
        ];
        let (out, report) = sanitize_tool_messages(msgs);
        assert_eq!(report.dropped_tool_messages, 1);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["role"], "user");
        assert!(!report.is_clean());
    }
}

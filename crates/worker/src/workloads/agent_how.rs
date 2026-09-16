//! Agent-kind (`kind=agent`) session-execution extras, kept pure so the
//! journal-free parts are unit-testable: the workflow-declared `how_append`
//! payload, the bounded output extraction and the result/title shapes.
//!
//! The operator/maintenance paths must stay byte-identical, so every helper
//! here is a no-op unless the execution kind is [`ExecutionKind::Agent`].

use anyhow::{bail, Result};
use opencoder_core::fleet::ExecutionKind;
use serde_json::{json, Value};

/// Agent sessions reuse the DAG agent step's `how_append` budget (UTF-8
/// bytes) — see `opencoder_dag::spec::MAX_HOW_APPEND_BYTES`.
pub(crate) const MAX_HOW_APPEND_BYTES: usize = opencoder_dag::spec::MAX_HOW_APPEND_BYTES;

/// Bounded transcript tail for output extraction — the DAG agent step's
/// 8 KiB tail rule (`MAX_TRANSCRIPT_TAIL` in `dag-runtime::exec::agent`).
pub(crate) const OUTPUT_TAIL_BYTES: usize = 8 * 1024;

/// The declared `how_append` payload for AGENT executions only; operator/
/// maintenance inputs ignore the field entirely. Oversized (or non-string)
/// payloads bail here even though the control plane already rejects them —
/// the node stays the admission authority.
pub(crate) fn declared_how_append(kind: ExecutionKind, input: &Value) -> Result<Option<String>> {
    if kind != ExecutionKind::Agent {
        return Ok(None);
    }
    match input.get("how_append") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.len() <= MAX_HOW_APPEND_BYTES => Ok(Some(text.clone())),
        Some(Value::String(text)) => bail!(
            "how_append exceeds {} bytes (got {})",
            MAX_HOW_APPEND_BYTES,
            text.len()
        ),
        Some(other) => bail!("how_append must be a string, got {other}"),
    }
}

/// Session title for a fresh agent-family execution: the caller's explicit
/// title, then the family default. Agent-kind sessions are plain chat rows —
/// unlike operator/maintenance they default to untitled.
pub(crate) fn default_title(kind: ExecutionKind, from_input: Option<&str>) -> Option<String> {
    from_input.map(str::to_owned).or_else(|| match kind {
        ExecutionKind::Maintenance => Some("节点维护".into()),
        ExecutionKind::Operator => Some("Operator".into()),
        _ => None,
    })
}

/// Trim to the last `max` UTF-8 bytes on a char boundary (the DAG agent
/// step's `push_tail` trim rule).
pub(crate) fn transcript_tail(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut cut = text.len() - max;
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    text[cut..].to_string()
}

/// Terminal result for agent-kind sessions: the session pointer plus the
/// DAG StepResult-shaped output contract. An empty transcript yields an
/// empty `output_text` and a JSON `null` (the DAG step's empty-transcript
/// semantics), never a missing key.
pub(crate) fn agent_result(id: &str, text: &str, output_json: Option<Value>) -> Value {
    json!({
        "session_id": id,
        "output_text": text,
        "output_json": output_json,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn how_append_is_agent_only_and_size_bounded() {
        let input = json!({"how_append": "note"});
        // Non-agent kinds ignore the field entirely.
        assert_eq!(
            declared_how_append(ExecutionKind::Operator, &input).unwrap(),
            None
        );
        assert_eq!(
            declared_how_append(ExecutionKind::Maintenance, &input).unwrap(),
            None
        );
        // Agent kind: string passthrough within the budget.
        assert_eq!(
            declared_how_append(ExecutionKind::Agent, &input).unwrap(),
            Some("note".into())
        );
        assert_eq!(
            declared_how_append(ExecutionKind::Agent, &json!({})).unwrap(),
            None
        );
        assert_eq!(
            declared_how_append(ExecutionKind::Agent, &json!({"how_append": null})).unwrap(),
            None
        );
        // Exactly at the budget passes; sizes measure UTF-8 bytes.
        let at_limit = "a".repeat(MAX_HOW_APPEND_BYTES);
        assert_eq!(
            declared_how_append(ExecutionKind::Agent, &json!({"how_append": at_limit}))
                .unwrap()
                .as_deref(),
            Some(at_limit.as_str())
        );
        // One byte over bails.
        let oversized = format!("{at_limit}x");
        assert!(
            declared_how_append(ExecutionKind::Agent, &json!({"how_append": oversized})).is_err()
        );
        // Non-string payloads bail instead of being silently dropped.
        assert!(declared_how_append(ExecutionKind::Agent, &json!({"how_append": 3})).is_err());
    }

    #[test]
    fn titles_follow_the_caller_then_the_kind_default() {
        assert_eq!(
            default_title(ExecutionKind::Agent, Some("t")),
            Some("t".into())
        );
        assert_eq!(default_title(ExecutionKind::Agent, None), None);
        assert_eq!(
            default_title(ExecutionKind::Operator, None),
            Some("Operator".into())
        );
        assert_eq!(
            default_title(ExecutionKind::Maintenance, None),
            Some("节点维护".into())
        );
        // The explicit title wins over the kind default.
        assert_eq!(
            default_title(ExecutionKind::Operator, Some("chat")),
            Some("chat".into())
        );
    }

    #[test]
    fn transcript_tail_keeps_the_char_safe_end() {
        assert_eq!(transcript_tail("short", 8), "short");
        let long = "a".repeat(20);
        assert_eq!(transcript_tail(&long, 8), "a".repeat(8));
        // Multibyte boundary: never splits a UTF-8 char.
        let chars = "字字字字字"; // 3 bytes each = 15 bytes
        assert_eq!(transcript_tail(chars, 7), "字字");
    }

    #[test]
    fn agent_result_carries_every_key_even_without_output() {
        let empty = agent_result("agent-1", "", None);
        assert_eq!(
            empty,
            json!({"session_id": "agent-1", "output_text": "", "output_json": null})
        );
        let full = agent_result("agent-1", "hello", Some(json!({"answer": 2})));
        assert_eq!(full["output_text"], "hello");
        assert_eq!(full["output_json"]["answer"], 2);
    }
}

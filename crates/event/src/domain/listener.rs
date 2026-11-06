//! Listener specs: which event kinds a listener reacts to and what it does.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::event::EventKind;

/// Catch-all pattern matching every kind.
pub const ALL_KINDS_PATTERN: &str = "*";

/// What to do when a listener's kind pattern matches a published event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerAction {
    /// Resume a paused execution and append `append` to its input.
    ResumeExecution { execution_id: String, append: Value },
    /// Wake a stalled decision loop at the given generation.
    WakeBrainDecision { run_id: String, generation: u64 },
    /// Emit a follow-up event through the sink the executor holds.
    EmitEvent { kind: EventKind, payload: Value },
    /// Cancel an execution.
    CancelExecution { execution_id: String },
    /// Surface a message to operators.
    Notify { message: String },
}

/// A registered listener: kind pattern plus trigger action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ListenerSpec {
    pub id: String,
    /// Exact kind or trailing-`*` prefix glob, e.g. `execution.*`.
    pub kind_pattern: String,
    pub enabled: bool,
    /// Optional scoping hint; semantics belong to the executor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub trigger: TriggerAction,
}

/// Builds a spec (enabled by default); pure constructor, no validation here.
pub fn new_listener(
    id: impl Into<String>,
    kind_pattern: impl Into<String>,
    scope: Option<String>,
    trigger: TriggerAction,
) -> ListenerSpec {
    ListenerSpec {
        id: id.into(),
        kind_pattern: kind_pattern.into(),
        enabled: true,
        scope,
        trigger,
    }
}

/// Validation failures for a [`ListenerSpec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ListenerError {
    #[error("listener id must be non-empty ascii without whitespace")]
    InvalidId,
    #[error(
        "kind pattern must be non-empty ascii; '*' is allowed at most once, as the final character"
    )]
    InvalidKindPattern,
    #[error("scope must be non-empty when present")]
    InvalidScope,
    #[error("trigger field must be non-empty: {0}")]
    InvalidTrigger(&'static str),
}

/// Validates a listener id: non-empty, ascii, no whitespace.
pub fn validate_id(id: &str) -> Result<(), ListenerError> {
    if !id.is_empty() && id.is_ascii() && !id.chars().any(char::is_whitespace) {
        Ok(())
    } else {
        Err(ListenerError::InvalidId)
    }
}

/// Validates a kind pattern: an exact kind, or `<prefix>*` prefix glob.
pub fn validate_kind_pattern(pattern: &str) -> Result<(), ListenerError> {
    let star_count = pattern.matches('*').count();
    let ok = !pattern.is_empty()
        && pattern.is_ascii()
        && !pattern.chars().any(char::is_whitespace)
        && star_count <= 1
        && (star_count == 0 || pattern.ends_with('*'));
    if ok {
        Ok(())
    } else {
        Err(ListenerError::InvalidKindPattern)
    }
}

/// Validates a full spec: id, kind pattern, scope and trigger fields.
pub fn validate(spec: &ListenerSpec) -> Result<(), ListenerError> {
    validate_id(&spec.id)?;
    validate_kind_pattern(&spec.kind_pattern)?;
    if let Some(scope) = &spec.scope {
        if scope.trim().is_empty() {
            return Err(ListenerError::InvalidScope);
        }
    }
    validate_trigger(&spec.trigger)
}

/// Id-like fields must be non-empty and free of whitespace (tokens).
fn require_token(value: &str, field: &'static str) -> Result<(), ListenerError> {
    if !value.is_empty() && !value.chars().any(char::is_whitespace) {
        Ok(())
    } else {
        Err(ListenerError::InvalidTrigger(field))
    }
}

/// Free-form fields (e.g. notification text) only need to be non-blank.
fn require_non_blank(value: &str, field: &'static str) -> Result<(), ListenerError> {
    if !value.trim().is_empty() {
        Ok(())
    } else {
        Err(ListenerError::InvalidTrigger(field))
    }
}

fn validate_trigger(trigger: &TriggerAction) -> Result<(), ListenerError> {
    match trigger {
        TriggerAction::ResumeExecution { execution_id, .. }
        | TriggerAction::CancelExecution { execution_id } => {
            require_token(execution_id, "execution_id")
        }
        TriggerAction::WakeBrainDecision { run_id, .. } => require_token(run_id, "run_id"),
        TriggerAction::EmitEvent { kind, .. } => {
            require_token(kind.as_str(), "kind")?;
            if kind.as_str().contains('*') {
                return Err(ListenerError::InvalidTrigger("kind"));
            }
            Ok(())
        }
        TriggerAction::Notify { message } => require_non_blank(message, "message"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str, pattern: &str) -> ListenerSpec {
        new_listener(
            id,
            pattern,
            None,
            TriggerAction::Notify {
                message: "m".to_string(),
            },
        )
    }

    #[test]
    fn validate_accepts_minimal_spec() {
        assert_eq!(validate(&spec("l1", "execution.*")), Ok(()));
        // notification text may contain whitespace
        assert_eq!(
            validate(&new_listener(
                "l1b",
                "execution.*",
                None,
                TriggerAction::Notify {
                    message: "saw l1b".to_string()
                }
            )),
            Ok(())
        );
        assert_eq!(
            validate(&new_listener(
                "l2",
                "brain.operation_terminal",
                Some("scope-a".to_string()),
                TriggerAction::WakeBrainDecision {
                    run_id: "run-1".to_string(),
                    generation: 3
                }
            )),
            Ok(())
        );
    }

    #[test]
    fn validate_rejects_bad_ids_patterns_and_scopes() {
        assert_eq!(validate(&spec("", "*")), Err(ListenerError::InvalidId));
        assert_eq!(validate(&spec("l 1", "*")), Err(ListenerError::InvalidId));
        assert_eq!(
            validate(&spec("l1", "execution.*.paused")),
            Err(ListenerError::InvalidKindPattern)
        );
        assert_eq!(
            validate(&spec("l1", "*execution")),
            Err(ListenerError::InvalidKindPattern)
        );
        let mut scoped = spec("l1", "*");
        scoped.scope = Some("   ".to_string());
        assert_eq!(validate(&scoped), Err(ListenerError::InvalidScope));
    }

    #[test]
    fn validate_rejects_empty_trigger_fields() {
        let cases = [
            TriggerAction::CancelExecution {
                execution_id: String::new(),
            },
            TriggerAction::ResumeExecution {
                execution_id: " ".to_string(),
                append: Value::Null,
            },
            TriggerAction::WakeBrainDecision {
                run_id: String::new(),
                generation: 1,
            },
            TriggerAction::EmitEvent {
                kind: EventKind::new("execution.*"),
                payload: Value::Null,
            },
            TriggerAction::Notify {
                message: String::new(),
            },
        ];
        for trigger in cases {
            let spec = new_listener("l1", "*", None, trigger);
            assert!(validate(&spec).is_err());
        }
    }

    #[test]
    fn trigger_action_serializes_tagged_variants() {
        let action = TriggerAction::ResumeExecution {
            execution_id: "exec-1".to_string(),
            append: serde_json::json!({"note": "ready"}),
        };
        let text = serde_json::to_string(&action).unwrap();
        assert!(text.contains("\"type\":\"resume_execution\""));
        assert!(text.contains("\"execution_id\":\"exec-1\""));
        let back: TriggerAction = serde_json::from_str(&text).unwrap();
        assert_eq!(back, action);
    }
}

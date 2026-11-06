//! Event kind vocabulary and the immutable [`EventRecord`].
//!
//! Kinds use an open `namespace.name` vocabulary: the well-known kinds below
//! cover platform scheduling signals, and callers may extend the space with
//! `event.custom.<x>` kinds via [`custom_kind`]. The kind is a newtype over
//! `String` (not a closed enum) so new signals never require a crate change.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An orchestration layer finished and its results await whole-layer
/// assessment.
pub const BRAIN_LAYER_BARRIER_REACHED: &str = "brain.layer_barrier_reached";
/// A brain run was paused: future scheduling stops while started children
/// keep running, and pending intents survive for an explicit resume.
pub const BRAIN_RUN_PAUSED: &str = "brain.run_paused";
/// A paused or blocked brain run was explicitly resumed.
pub const BRAIN_RUN_RESUMED: &str = "brain.run_resumed";
/// A scheduled operation reached a terminal state.
pub const BRAIN_OPERATION_TERMINAL: &str = "brain.operation_terminal";
/// An execution container reached a terminal state.
pub const EXECUTION_TERMINAL: &str = "execution.terminal";
/// An execution carrying an actual execution id was paused and can be
/// resumed; run-level brain pauses use [`BRAIN_RUN_PAUSED`] instead.
pub const EXECUTION_PAUSED: &str = "execution.paused";
/// Prefix for caller-defined kinds: `event.custom.<x>`.
pub const CUSTOM_KIND_PREFIX: &str = "event.custom.";

/// Open-vocabulary event kind.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventKind(pub String);

impl EventKind {
    /// Builds a kind from any string; callers pick names from the open
    /// vocabulary, e.g. [`EXECUTION_TERMINAL`] or [`custom_kind`].
    pub fn new(kind: impl Into<String>) -> Self {
        Self(kind.into())
    }

    /// The kind text, e.g. `execution.terminal`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for EventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Errors raised while building caller-defined kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EventKindError {
    /// The suffix after `event.custom.` was empty or blank.
    #[error("custom kind name must be non-empty")]
    EmptyName,
    /// The suffix contained characters outside `[A-Za-z0-9._-]`.
    #[error("custom kind name may only contain ascii letters, digits, '.', '_' and '-'")]
    InvalidCharacter,
}

/// Builds a caller-defined kind: `event.custom.<name>`.
pub fn custom_kind(name: &str) -> Result<EventKind, EventKindError> {
    if name.trim().is_empty() {
        return Err(EventKindError::EmptyName);
    }
    let valid = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
    if !valid {
        return Err(EventKindError::InvalidCharacter);
    }
    Ok(EventKind::new(format!("{CUSTOM_KIND_PREFIX}{name}")))
}

/// Returns true when the kind lives in the caller-defined `event.custom.*`
/// space.
pub fn is_custom(kind: &EventKind) -> bool {
    kind.as_str().starts_with(CUSTOM_KIND_PREFIX)
}

/// A single immutable event: what happened, who emitted it, and the sequence
/// number used as the replay cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EventRecord {
    /// Unique record id (ulid).
    pub id: String,
    /// What happened.
    pub kind: EventKind,
    /// Kind-specific payload.
    pub payload: Value,
    /// Emitter identity, e.g. a component or node name.
    pub source: String,
    /// Sequence number; the ledger orders and cursors on this field.
    pub seq: i64,
    /// Emission time; injected by the caller for testability.
    pub occurred_at: DateTime<Utc>,
}

/// Fresh unique record id (ulid).
pub fn new_id() -> String {
    ulid::Ulid::new().to_string()
}

/// Builds a new record: generates the ulid id, keeps the injected clock time.
pub fn new_record(
    kind: EventKind,
    payload: Value,
    source: impl Into<String>,
    seq: i64,
    now: DateTime<Utc>,
) -> EventRecord {
    EventRecord {
        id: new_id(),
        kind,
        payload,
        source: source.into(),
        seq,
        occurred_at: now,
    }
}

/// Rebuilds a record from persisted fields (no id or clock generation).
pub fn rebuild_record(
    id: impl Into<String>,
    kind: EventKind,
    payload: Value,
    source: impl Into<String>,
    seq: i64,
    occurred_at: DateTime<Utc>,
) -> EventRecord {
    EventRecord {
        id: id.into(),
        kind,
        payload,
        source: source.into(),
        seq,
        occurred_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_kind_builds_prefixed_kind_and_rejects_bad_names() {
        let kind = custom_kind("metric.collected").unwrap();
        assert_eq!(kind.as_str(), "event.custom.metric.collected");
        assert!(is_custom(&kind));
        assert!(!is_custom(&EventKind::new(EXECUTION_PAUSED)));
        assert_eq!(custom_kind(""), Err(EventKindError::EmptyName));
        assert_eq!(custom_kind("  "), Err(EventKindError::EmptyName));
        assert_eq!(custom_kind("a*b"), Err(EventKindError::InvalidCharacter));
        assert_eq!(custom_kind("a b"), Err(EventKindError::InvalidCharacter));
    }

    #[test]
    fn record_serializes_snake_case_with_transparent_kind() {
        let now = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
        let record = new_record(
            EventKind::new(EXECUTION_PAUSED),
            serde_json::json!({"node_id": "n1"}),
            "scheduler",
            7,
            now,
        );
        let text = serde_json::to_string(&record).unwrap();
        assert!(text.contains("\"kind\":\"execution.paused\""));
        assert!(text.contains("\"occurred_at\""));
        assert!(text.contains("\"seq\":7"));
        let back: EventRecord = serde_json::from_str(&text).unwrap();
        assert_eq!(back, record);
        assert_eq!(back.id.len(), 26); // ulid text length
    }
}

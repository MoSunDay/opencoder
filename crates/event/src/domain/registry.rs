//! Pure listener-registry math over an immutable `Vec<ListenerSpec>`.
//!
//! Every function takes the current registry by reference and returns the
//! next registry value; inputs are never mutated, so callers own the
//! update (write it back through the ledger when needed).

use crate::domain::event::EventKind;
use crate::domain::listener::{validate, ListenerError, ListenerSpec};

/// Registry update failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    /// The spec failed domain validation.
    #[error(transparent)]
    InvalidSpec(#[from] ListenerError),
    /// A listener with the same id is already registered.
    #[error("listener id already registered: {0}")]
    DuplicateId(String),
}

/// Validates `spec` and returns the registry with it appended; ids must be
/// unique.
pub fn register(
    registry: &[ListenerSpec],
    spec: ListenerSpec,
) -> Result<Vec<ListenerSpec>, RegistryError> {
    validate(&spec)?;
    if registry.iter().any(|listener| listener.id == spec.id) {
        return Err(RegistryError::DuplicateId(spec.id));
    }
    let mut next = registry.to_vec();
    next.push(spec);
    Ok(next)
}

/// Returns the registry without the listener `id`; unknown ids are no-ops.
pub fn remove(registry: &[ListenerSpec], id: &str) -> Vec<ListenerSpec> {
    registry
        .iter()
        .filter(|listener| listener.id != id)
        .cloned()
        .collect()
}

/// Returns the registry with `id`'s `enabled` flag set; unknown ids are
/// no-ops.
pub fn set_enabled(registry: &[ListenerSpec], id: &str, enabled: bool) -> Vec<ListenerSpec> {
    registry
        .iter()
        .map(|listener| {
            if listener.id == id {
                let mut updated = listener.clone();
                updated.enabled = enabled;
                updated
            } else {
                listener.clone()
            }
        })
        .collect()
}

/// Enabled listeners whose pattern matches `kind`, in registry order.
pub fn listeners_for(registry: &[ListenerSpec], kind: &EventKind) -> Vec<ListenerSpec> {
    registry
        .iter()
        .filter(|listener| listener.enabled && matches(&listener.kind_pattern, kind.as_str()))
        .cloned()
        .collect()
}

/// Pattern match: exact equality, or prefix glob when the pattern ends with
/// `*` (a lone `*` matches every kind).
pub fn matches(pattern: &str, kind: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => kind.starts_with(prefix),
        None => pattern == kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::listener::{new_listener, TriggerAction};

    fn listener(id: &str, pattern: &str) -> ListenerSpec {
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
    fn matches_exact_and_glob_prefixes() {
        assert!(matches("execution.paused", "execution.paused"));
        assert!(!matches("execution.paused", "execution.terminal"));
        assert!(matches("execution.*", "execution.terminal"));
        assert!(matches("*", "anything.here"));
        assert!(!matches("execution.*", "execution"));
        assert!(!matches("execution.*", "brain.operation_terminal"));
    }

    #[test]
    fn register_rejects_duplicates_and_keeps_input_intact() {
        let base = vec![listener("dup", "execution.*")];
        let err = register(&base, listener("dup", "brain.*")).unwrap_err();
        assert_eq!(err, RegistryError::DuplicateId("dup".to_string()));
        assert_eq!(base.len(), 1);
        let next = register(&base, listener("fresh", "brain.*")).unwrap();
        assert_eq!(next.len(), 2);
        assert_eq!(base.len(), 1);
    }

    #[test]
    fn remove_and_set_enabled_return_new_registries() {
        let base = vec![listener("l1", "execution.*"), listener("l2", "brain.*")];
        assert_eq!(remove(&base, "l1")[0].id, "l2");
        assert_eq!(remove(&base, "missing").len(), 2);
        let paused = set_enabled(&base, "l2", false);
        assert!(!paused[1].enabled);
        assert!(base[1].enabled);
        // unknown id: no change
        assert!(set_enabled(&base, "missing", false)[0].enabled);
    }
}

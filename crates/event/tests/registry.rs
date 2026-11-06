//! Registry behavior: pattern matching, immutable updates and validation.

use opencoder_event::domain::event::EventKind;
use opencoder_event::domain::listener::{new_listener, ListenerError, TriggerAction};
use opencoder_event::domain::registry::{self, RegistryError};

fn notify_listener(id: &str, pattern: &str) -> opencoder_event::domain::listener::ListenerSpec {
    new_listener(
        id,
        pattern,
        None,
        TriggerAction::Notify {
            message: format!("saw {id}"),
        },
    )
}

#[test]
fn exact_pattern_matches_only_that_kind() {
    assert!(registry::matches("execution.paused", "execution.paused"));
    assert!(!registry::matches("execution.paused", "execution.terminal"));
    assert!(!registry::matches("execution.paused", "execution"));
}

#[test]
fn prefix_glob_matches_prefixed_kinds_only() {
    assert!(registry::matches("execution.*", "execution.terminal"));
    assert!(registry::matches("execution.*", "execution.paused"));
    assert!(registry::matches("execution.*", "execution.paused.later"));
    assert!(!registry::matches("execution.*", "execution"));
    assert!(!registry::matches(
        "execution.*",
        "brain.operation_terminal"
    ));
}

#[test]
fn catch_all_pattern_matches_every_kind() {
    assert!(registry::matches("*", "execution.terminal"));
    assert!(registry::matches("*", "event.custom.anything"));
}

#[test]
fn register_rejects_invalid_glob_patterns() {
    for pattern in [
        "",
        "   ",
        "execution.*.paused",
        "execution.**",
        "*execution",
        "ex* ",
    ] {
        let spec = notify_listener("l1", pattern);
        assert_eq!(
            registry::register(&[], spec).unwrap_err(),
            RegistryError::InvalidSpec(ListenerError::InvalidKindPattern),
            "pattern {pattern:?} must be rejected"
        );
    }
}

#[test]
fn register_rejects_invalid_ids_and_triggers() {
    assert_eq!(
        registry::register(&[], notify_listener("", "*")).unwrap_err(),
        RegistryError::InvalidSpec(ListenerError::InvalidId)
    );
    assert_eq!(
        registry::register(&[], notify_listener("l 1", "*")).unwrap_err(),
        RegistryError::InvalidSpec(ListenerError::InvalidId)
    );
    let bad_trigger = new_listener(
        "l1",
        "*",
        None,
        TriggerAction::CancelExecution {
            execution_id: String::new(),
        },
    );
    assert!(registry::register(&[], bad_trigger).is_err());
}

#[test]
fn register_rejects_duplicate_id_without_touching_the_input() {
    let base = vec![notify_listener("dup", "execution.*")];
    let err = registry::register(&base, notify_listener("dup", "brain.*")).unwrap_err();
    assert_eq!(err, RegistryError::DuplicateId("dup".to_string()));
    assert_eq!(base.len(), 1);
}

#[test]
fn remove_drops_only_the_target_listener() {
    let base = vec![
        notify_listener("l1", "execution.*"),
        notify_listener("l2", "brain.*"),
    ];
    let next = registry::remove(&base, "l1");
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].id, "l2");
    assert_eq!(registry::remove(&base, "missing").len(), 2);
    assert_eq!(base.len(), 2);
}

#[test]
fn set_enabled_excludes_listener_from_matching_but_keeps_it_listed() {
    let base = vec![notify_listener("l1", "execution.*")];
    let paused = registry::set_enabled(&base, "l1", false);
    assert!(!paused[0].enabled);
    let kind = EventKind::new("execution.terminal");
    assert_eq!(registry::listeners_for(&base, &kind).len(), 1);
    assert!(registry::listeners_for(&paused, &kind).is_empty());
    // unknown id is a no-op
    let unchanged = registry::set_enabled(&paused, "nope", true);
    assert_eq!(unchanged.len(), 1);
    assert!(!unchanged[0].enabled);
}

#[test]
fn listeners_for_keeps_registry_order_across_matching_patterns() {
    let disabled = {
        let one = vec![notify_listener("l4", "execution.*")];
        registry::set_enabled(&one, "l4", false).remove(0)
    };
    let reg = vec![
        notify_listener("l1", "execution.terminal"),
        notify_listener("l2", "execution.*"),
        notify_listener("l3", "brain.*"),
        disabled,
    ];
    let hits = registry::listeners_for(&reg, &EventKind::new("execution.terminal"));
    let ids: Vec<&str> = hits.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(ids, ["l1", "l2"]);
    assert_eq!(hits[1].kind_pattern, "execution.*");
}

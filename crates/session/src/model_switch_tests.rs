use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ConfigMode};

use super::*;
use crate::SessionState;

fn session() -> SessionState {
    let dir = tempfile::tempdir().unwrap();
    SessionState::new(
        "model-switch-queue",
        resolve_agent("act").unwrap(),
        Config {
            model: "provider/before".into(),
            ..Config::default()
        },
        Arc::new(opencoder_llm::MockChatClient::new()),
        dir.path().to_path_buf(),
    )
}

#[test]
fn queue_keeps_submission_order_and_drains_once() {
    let queue = ModelSwitchQueue::default();
    assert_eq!(pending_model_switches(&queue), 0);
    queue_model_switch(&queue, ConfigMode::Act, "provider/one");
    queue_model_switch(&queue, ConfigMode::Plan, "provider/two");
    let shared = queue.clone();
    queue_model_switch(&shared, ConfigMode::Act, "provider/three");
    assert_eq!(pending_model_switches(&queue), 3);
    let picks = drain_model_switches(&queue);
    assert_eq!(
        picks,
        vec![
            (ConfigMode::Act, "provider/one".to_string()),
            (ConfigMode::Plan, "provider/two".to_string()),
            (ConfigMode::Act, "provider/three".to_string()),
        ]
    );
    assert!(drain_model_switches(&queue).is_empty());
    assert_eq!(pending_model_switches(&queue), 0);
}

#[test]
fn mode_keys_roundtrip() {
    for mode in [ConfigMode::Shared, ConfigMode::Act, ConfigMode::Plan] {
        assert_eq!(mode_from_key(mode_key(mode)), Some(mode));
    }
    assert_eq!(mode_from_key("Act"), None);
}

/// An empty queue is a silent no-op: the runner calls this after every LLM
/// round, so the common case must emit nothing.
#[tokio::test]
async fn no_pending_pick_emits_nothing() {
    let mut session = session();
    let mut events = Vec::new();
    apply_queued_model_switches(&mut session, &mut |event| events.push(event)).await;
    assert!(events.is_empty());
    assert_eq!(session.effective_model(), "provider/before");
}

#[tokio::test]
async fn applied_pick_switches_the_effective_model_and_reports_its_mode() {
    let mut session = session();
    let queue = session.model_switch_queue.clone();
    queue_model_switch(&queue, ConfigMode::Act, "provider/after");
    let mut events = Vec::new();
    apply_queued_model_switches(&mut session, &mut |event| events.push(event)).await;

    assert_eq!(session.effective_model(), "provider/after");
    assert_eq!(
        session.model_override(ConfigMode::Act),
        Some("provider/after")
    );
    assert_eq!(
        pending_model_switches(&queue),
        0,
        "the runner drained the queue"
    );
    assert!(matches!(
        events.as_slice(),
        [
            SessionEvent::ModelSwitch(model),
            SessionEvent::ModelOverrideApplied {
                mode,
                model: applied,
                error: None
            }
        ] if model == "after" && mode == "act" && applied == "provider/after"
    ));
}

/// A pick for a mode the current agent does not use changes only that mode's
/// selection: no `ModelSwitch` (the next request keeps its model), but the
/// applied pick is still reported so the display marks the mode's new model.
#[tokio::test]
async fn pick_for_the_other_mode_reports_without_a_model_switch() {
    let mut session = session();
    let queue = session.model_switch_queue.clone();
    queue_model_switch(&queue, ConfigMode::Plan, "provider/plan-pick");
    let mut events = Vec::new();
    apply_queued_model_switches(&mut session, &mut |event| events.push(event)).await;

    assert_eq!(session.effective_model(), "provider/before");
    assert_eq!(
        session.model_override(ConfigMode::Plan),
        Some("provider/plan-pick")
    );
    assert!(matches!(
        events.as_slice(),
        [SessionEvent::ModelOverrideApplied { mode, model, error: None }]
            if mode == "plan" && model == "provider/plan-pick"
    ));
}

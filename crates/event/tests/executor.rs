//! Dispatch planning and execution behavior.

use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};
use opencoder_event::domain::event::{new_record, EventKind, EventRecord};
use opencoder_event::domain::listener::{new_listener, ListenerSpec, TriggerAction};
use opencoder_event::domain::registry;
use opencoder_event::executor::{self, ActionExecutor};
use serde_json::json;

fn event(kind: &str) -> EventRecord {
    let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    new_record(EventKind::new(kind), json!({}), "test", 1, now)
}

fn action_tag(action: &TriggerAction) -> &'static str {
    match action {
        TriggerAction::ResumeExecution { .. } => "resume_execution",
        TriggerAction::WakeBrainDecision { .. } => "wake_brain_decision",
        TriggerAction::EmitEvent { .. } => "emit_event",
        TriggerAction::CancelExecution { .. } => "cancel_execution",
        TriggerAction::Notify { .. } => "notify",
    }
}

/// Records (action tag, event id) pairs; optionally fails one action tag.
struct RecordingExecutor {
    calls: Arc<Mutex<Vec<(String, String)>>>,
    fail_tag: Option<&'static str>,
}

#[async_trait::async_trait]
impl ActionExecutor for RecordingExecutor {
    async fn execute(&self, action: &TriggerAction, event: &EventRecord) -> anyhow::Result<()> {
        let tag = action_tag(action);
        if self.fail_tag == Some(tag) {
            anyhow::bail!("planned failure for {tag}");
        }
        self.calls
            .lock()
            .unwrap()
            .push((tag.to_string(), event.id.clone()));
        Ok(())
    }
}

fn notify(id: &str, pattern: &str) -> ListenerSpec {
    new_listener(
        id,
        pattern,
        None,
        TriggerAction::Notify {
            message: format!("m-{id}"),
        },
    )
}

#[test]
fn plan_selects_enabled_matching_listeners_in_order() {
    let disabled = registry::set_enabled(&[notify("l4", "execution.*")], "l4", false).remove(0);
    let reg = vec![
        notify("l1", "execution.terminal"),
        notify("l2", "execution.*"),
        notify("l3", "brain.*"),
        disabled,
    ];
    let planned = executor::plan(&reg, &event("execution.terminal"));
    let ids: Vec<&str> = planned.iter().map(|(spec, _)| spec.id.as_str()).collect();
    assert_eq!(ids, ["l1", "l2"]);
    assert_eq!(
        planned[0].1,
        TriggerAction::Notify {
            message: "m-l1".to_string()
        }
    );
    assert!(planned
        .iter()
        .all(|(_, action)| action_tag(action) == "notify"));
}

#[tokio::test]
async fn dispatch_runs_every_planned_action_once() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let executor = RecordingExecutor {
        calls: Arc::clone(&calls),
        fail_tag: None,
    };
    let reg = vec![
        notify("l1", "execution.*"),
        new_listener(
            "l2",
            "execution.terminal",
            None,
            TriggerAction::EmitEvent {
                kind: EventKind::new("event.custom.followup"),
                payload: json!({"origin": "l2"}),
            },
        ),
        notify("l3", "brain.*"),
    ];

    let evt = event("execution.terminal");
    let outcomes = executor::dispatch(&executor, &reg, &evt).await;
    assert_eq!(outcomes.len(), 2);
    assert_eq!(executor::success_count(&outcomes), 2);
    assert!(executor::failure_ids(&outcomes).is_empty());
    assert_eq!(
        outcomes
            .iter()
            .map(|o| o.listener_id.as_str())
            .collect::<Vec<_>>(),
        ["l1", "l2"]
    );

    let recorded = calls.lock().unwrap().clone();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0], ("notify".to_string(), evt.id.clone()));
    assert_eq!(recorded[1], ("emit_event".to_string(), evt.id));
}

#[tokio::test]
async fn dispatch_continues_after_failure_and_reports_per_listener_results() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let executor = RecordingExecutor {
        calls: Arc::clone(&calls),
        fail_tag: Some("cancel_execution"),
    };
    let reg = vec![
        notify("l1", "execution.*"),
        new_listener(
            "l2",
            "execution.*",
            None,
            TriggerAction::CancelExecution {
                execution_id: "exec-1".to_string(),
            },
        ),
        notify("l3", "execution.*"),
    ];

    let outcomes = executor::dispatch(&executor, &reg, &event("execution.paused")).await;
    assert_eq!(outcomes.len(), 3);
    assert_eq!(executor::success_count(&outcomes), 2);
    assert_eq!(executor::failure_ids(&outcomes), ["l2"]);
    assert!(outcomes[1].result.is_err());
    assert!(outcomes[2].result.is_ok());

    // l3 still ran after l2 failed
    let recorded = calls.lock().unwrap();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].0, "notify");
    assert_eq!(recorded[1].0, "notify");
}

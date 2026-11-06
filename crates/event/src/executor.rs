//! Trigger execution: plan which listeners react, then run their actions.
//!
//! Planning ([`plan`]) is pure and unit-testable; execution ([`dispatch`],
//! [`execute_planned`]) walks the plan sequentially and reports one outcome
//! per listener, so a failing action never stops the remaining ones.
//! Side-effect targets (execution control, decision loop wake, notification
//! sink) belong to the [`ActionExecutor`] implementation; `EmitEvent`
//! actions are published through a sink the executor holds (e.g. an
//! [`crate::bus::EventBus`] handle plus ledger), keeping dispatch itself
//! sink-free.

use async_trait::async_trait;

#[cfg(test)]
use crate::domain::event::EventKind;
use crate::domain::event::EventRecord;
use crate::domain::gate::{allows_dispatch, SchedulerGate};
use crate::domain::listener::{ListenerSpec, TriggerAction};
use crate::domain::registry::listeners_for;

/// Executes one trigger action in the context of the event that fired it.
#[async_trait]
pub trait ActionExecutor: Send + Sync {
    async fn execute(&self, action: &TriggerAction, event: &EventRecord) -> anyhow::Result<()>;
}

/// A planned listener reaction: the spec and the action it will run.
pub type PlannedAction = (ListenerSpec, TriggerAction);

/// Pure planning: enabled listeners whose pattern matches the event kind, in
/// registry order, each paired with its trigger action.
pub fn plan(registry: &[ListenerSpec], event: &EventRecord) -> Vec<PlannedAction> {
    listeners_for(registry, &event.kind)
        .into_iter()
        .map(|spec| {
            let action = spec.trigger.clone();
            (spec, action)
        })
        .collect()
}

/// Per-listener result of a dispatch run.
#[derive(Debug)]
pub struct DispatchOutcome {
    pub listener_id: String,
    pub action: TriggerAction,
    pub result: anyhow::Result<()>,
}

/// Explicit verdict of a gated dispatch run.
///
/// `Paused` (gate blocked, nothing ran) is a different fact from
/// `Dispatched(vec!)` (gate open, no listener matched), so consumers can
/// decide whether to hold their replay cursor or move on.
#[derive(Debug)]
pub enum DispatchDecision {
    /// The gate blocked dispatch; no listener was consulted.
    Paused,
    /// The gate allowed dispatch; one outcome per planned listener.
    Dispatched(Vec<DispatchOutcome>),
}

impl DispatchDecision {
    /// True when the gate blocked this dispatch.
    pub fn is_paused(&self) -> bool {
        matches!(self, DispatchDecision::Paused)
    }

    /// Ran outcomes; empty for paused runs and for no-match runs — tell
    /// them apart with [`DispatchDecision::is_paused`].
    pub fn outcomes(&self) -> &[DispatchOutcome] {
        match self {
            DispatchDecision::Paused => &[],
            DispatchDecision::Dispatched(outcomes) => outcomes,
        }
    }
}

/// Runs every planned action sequentially; one failure never stops the rest.
pub async fn execute_planned(
    executor: &dyn ActionExecutor,
    planned: Vec<PlannedAction>,
    event: &EventRecord,
) -> Vec<DispatchOutcome> {
    let mut outcomes = Vec::with_capacity(planned.len());
    for (spec, action) in planned {
        let result = executor.execute(&action, event).await;
        outcomes.push(DispatchOutcome {
            listener_id: spec.id,
            action,
            result,
        });
    }
    outcomes
}

/// Plans and executes reactions for `event` in one step.
pub async fn dispatch(
    executor: &dyn ActionExecutor,
    registry: &[ListenerSpec],
    event: &EventRecord,
) -> Vec<DispatchOutcome> {
    let planned = plan(registry, event);
    execute_planned(executor, planned, event).await
}

/// Plans and executes only when the global gate allows dispatch. Paused
/// runs return [`DispatchDecision::Paused`]; callers keep appending to the
/// durable log and replay from their cursor after
/// [`crate::domain::gate::resume`].
pub async fn dispatch_gated(
    executor: &dyn ActionExecutor,
    registry: &[ListenerSpec],
    event: &EventRecord,
    gate: &SchedulerGate,
) -> DispatchDecision {
    if !allows_dispatch(gate) {
        return DispatchDecision::Paused;
    }
    DispatchDecision::Dispatched(dispatch(executor, registry, event).await)
}

/// Ids of listeners whose action failed, in dispatch order.
pub fn failure_ids(outcomes: &[DispatchOutcome]) -> Vec<&str> {
    outcomes
        .iter()
        .filter(|outcome| outcome.result.is_err())
        .map(|outcome| outcome.listener_id.as_str())
        .collect()
}

/// Number of successfully executed actions.
pub fn success_count(outcomes: &[DispatchOutcome]) -> usize {
    outcomes
        .iter()
        .filter(|outcome| outcome.result.is_ok())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(id: &str, ok: bool) -> DispatchOutcome {
        DispatchOutcome {
            listener_id: id.to_string(),
            action: TriggerAction::Notify {
                message: "m".to_string(),
            },
            result: if ok {
                Ok(())
            } else {
                Err(anyhow::anyhow!("planned failure"))
            },
        }
    }

    struct Counting(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    #[async_trait]
    impl ActionExecutor for Counting {
        async fn execute(
            &self,
            _action: &TriggerAction,
            _event: &EventRecord,
        ) -> anyhow::Result<()> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    fn listener() -> ListenerSpec {
        crate::domain::listener::new_listener(
            "l1",
            "*",
            None,
            TriggerAction::Notify {
                message: "m".to_string(),
            },
        )
    }

    fn event() -> EventRecord {
        crate::domain::event::new_record(
            EventKind::new("execution.terminal"),
            serde_json::Value::Null,
            "test",
            1,
            chrono::Utc::now(),
        )
    }

    #[tokio::test]
    async fn paused_gate_skips_dispatch_without_touching_listeners() {
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let executor = Counting(hits.clone());
        let closed = crate::domain::gate::paused("maintenance", 1);
        let decision = dispatch_gated(&executor, &[listener()], &event(), &closed).await;
        assert!(decision.is_paused());
        assert!(decision.outcomes().is_empty());
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);

        let open = crate::domain::gate::resumed(2);
        let decision = dispatch_gated(&executor, &[listener()], &event(), &open).await;
        assert!(!decision.is_paused());
        assert_eq!(decision.outcomes().len(), 1);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn open_gate_without_matching_listeners_is_dispatched_empty() {
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let executor = Counting(hits.clone());
        let open = crate::domain::gate::resumed(3);
        // No listeners at all: dispatched (empty), not paused.
        let decision = dispatch_gated(&executor, &[], &event(), &open).await;
        assert!(matches!(
            decision,
            DispatchDecision::Dispatched(ref outcomes) if outcomes.is_empty()
        ));
        assert!(!decision.is_paused());

        // Listeners exist but none match the kind: still dispatched (empty).
        let unrelated = crate::domain::listener::new_listener(
            "l9",
            "brain.*",
            None,
            TriggerAction::Notify {
                message: "m".to_string(),
            },
        );
        let decision = dispatch_gated(&executor, &[unrelated], &event(), &open).await;
        assert!(matches!(
            decision,
            DispatchDecision::Dispatched(ref outcomes) if outcomes.is_empty()
        ));
        assert!(!decision.is_paused());
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn outcome_helpers_count_successes_and_failures() {
        let outcomes = vec![
            outcome("l1", true),
            outcome("l2", false),
            outcome("l3", true),
        ];
        assert_eq!(success_count(&outcomes), 2);
        assert_eq!(failure_ids(&outcomes), ["l2"]);
    }
}

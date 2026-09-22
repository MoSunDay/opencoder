//! Collapse identical in-flight outbox deliveries. The durable node outbox
//! retries after errors or reconnects; this set owns no acknowledgement state.
use opencoder_core::fleet::ExecutionRef;
use serde_json::Value;
use std::{collections::HashSet, sync::Arc, sync::Mutex};

#[derive(Clone, Default)]
pub(super) struct InFlight(Arc<Mutex<HashSet<String>>>);

impl InFlight {
    pub(super) fn start(
        &self,
        execution: &ExecutionRef,
        action: &str,
        input: &Value,
    ) -> Option<Delivery> {
        let key =
            opencoder_core::token_hash(&serde_json::json!([execution, action, input]).to_string());
        let mut active = self.0.lock().unwrap();
        if active.len() >= 32 || !active.insert(key.clone()) {
            return None;
        }
        Some(Delivery {
            active: self.clone(),
            key,
        })
    }
}

pub(super) struct Delivery {
    active: InFlight,
    key: String,
}

impl Drop for Delivery {
    fn drop(&mut self) {
        self.active.0.lock().unwrap().remove(&self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::fleet::ExecutionKind;
    use serde_json::json;

    fn root() -> ExecutionRef {
        ExecutionRef {
            id: "brain-replay".into(),
            kind: ExecutionKind::Brain,
        }
    }

    #[test]
    fn duplicate_does_not_queue_but_later_generation_and_retry_are_allowed() {
        let active = InFlight::default();
        let first = active.start(&root(), "layered_wake", &json!({"generation":0}));
        assert!(first.is_some());
        for _ in 0..100 {
            assert!(active
                .start(&root(), "layered_wake", &json!({"generation":0}))
                .is_none());
        }
        let next = active.start(&root(), "layered_wake", &json!({"generation":4}));
        assert!(next.is_some());
        drop(first);
        assert!(active
            .start(&root(), "layered_wake", &json!({"generation":0}))
            .is_some());
    }

    #[tokio::test]
    async fn cancellation_releases_delivery_and_capacity_is_bounded() {
        let active = InFlight::default();
        let delivery = active.start(&root(), "layered_wake", &json!({})).unwrap();
        let task = tokio::spawn(async move {
            let _delivery = delivery;
            std::future::pending::<()>().await;
        });
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let guards: Vec<_> = (0..32)
            .map(|n| {
                active
                    .start(&root(), "layered_terminal", &json!(n))
                    .unwrap()
            })
            .collect();
        assert!(active.start(&root(), "layered_wake", &json!({})).is_none());
        drop(guards);
        assert!(active.start(&root(), "layered_wake", &json!({})).is_some());
    }
}

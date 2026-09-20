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
        let first = active.start(&root(), "scheduler_wake", &json!({"generation":0}));
        assert!(first.is_some());
        for _ in 0..100 {
            assert!(active
                .start(&root(), "scheduler_wake", &json!({"generation":0}))
                .is_none());
        }
        let next = active.start(&root(), "scheduler_wake", &json!({"generation":4}));
        assert!(next.is_some());
        drop(first);
        assert!(active
            .start(&root(), "scheduler_wake", &json!({"generation":0}))
            .is_some());
    }

    #[tokio::test]
    async fn cancellation_releases_delivery_and_capacity_is_bounded() {
        let active = InFlight::default();
        let delivery = active.start(&root(), "scheduler_wake", &json!({})).unwrap();
        let task = tokio::spawn(async move {
            let _delivery = delivery;
            std::future::pending::<()>().await;
        });
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let guards: Vec<_> = (0..32)
            .map(|n| {
                active
                    .start(&root(), "scheduler_terminal", &json!(n))
                    .unwrap()
            })
            .collect();
        assert!(active
            .start(&root(), "scheduler_wake", &json!({}))
            .is_none());
        drop(guards);
        assert!(active
            .start(&root(), "scheduler_wake", &json!({}))
            .is_some());
    }

    #[tokio::test]
    async fn old_wake_cannot_acknowledge_a_new_round_that_appears_during_delivery() {
        use crate::transport::SocketCommand;
        use opencoder_core::fleet::*;
        let directory = tempfile::tempdir().unwrap();
        let _home = opencoder_core::config::scoped_config_home(directory.path().join("home"));
        let state = crate::new_state(
            directory.path().join("work"),
            directory.path().join("data"),
            None,
        )
        .await
        .unwrap();
        let registration = NodeRegistration {
            protocol_version: PROTOCOL_VERSION,
            id: "node-wake-test".into(),
            name: "wake-test".into(),
            version: "test".into(),
            maintenance_agent_id: "maintenance-wake-test".into(),
            kinds: vec![ExecutionKind::Brain],
        };
        state.fleet.register(&registration).await.unwrap();
        state
            .fleet
            .put_index(&ExecutionIndex {
                id: root().id,
                kind: ExecutionKind::Brain,
                node_id: registration.id.clone(),
                created_at: 1,
                status: ExecutionStatus::Idle,
            })
            .await
            .unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        state
            .hub
            .attach(
                registration,
                NodeSnapshot {
                    pending_runs: 0,
                    queue_order: Default::default(),
                    generation: "g1".into(),
                    sequence: 1,
                    cpu_capacity: 1.0,
                    active_agent_loops: 0,
                    active_runs: 0,
                    max_runs: 1,
                    ready: true,
                    resource_error: None,
                },
                tx,
            )
            .await
            .unwrap();
        state.hub.mark_index_synced("node-wake-test", "g1").await;
        let acknowledgements = Arc::new(Mutex::new(Vec::new()));
        let received = acknowledgements.clone();
        let owner = state.clone();
        let node = tokio::spawn(async move {
            let mut snapshots = 0;
            while let Some(SocketCommand::Frame(frame)) = rx.recv().await {
                let ServerFrame::Call {
                    request_id,
                    operation,
                } = *frame;
                let NodeOperation::Brain { action, input, .. } = operation else {
                    panic!("unexpected operation")
                };
                let body = match action.as_str() {
                    "snapshot" => {
                        snapshots += 1;
                        // The terminal barrier commits after the first read.
                        json!({"schema_version":3,"operations":[],"run":{
                            "run_id":root().id,"phase":if snapshots == 1 {"waiting"} else {"ready"},
                            "generation":if snapshots == 1 {3} else {4}, "round":1,
                            "last_event_seq":7,"error":null,"created_at":1,"updated_at":2
                        }})
                    }
                    "scheduler_wake_ack" => {
                        received
                            .lock()
                            .unwrap()
                            .push(input["generation"].as_u64().unwrap());
                        json!({"acknowledged":input["generation"]})
                    }
                    _ => panic!("unexpected action {action}"),
                };
                owner
                    .hub
                    .resolve("node-wake-test", "g1", &request_id, RpcReply::ok(body))
                    .await;
            }
        });
        crate::api::brain_runs::v3::delivery::deliver(
            &state,
            "node-wake-test",
            &root(),
            "scheduler_wake",
            json!({"generation":1}),
        )
        .await
        .unwrap();
        assert_eq!(*acknowledgements.lock().unwrap(), vec![1]);
        node.abort();
        let _ = node.await;
    }
}

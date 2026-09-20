use crate::transport::SocketCommand;
use opencoder_core::fleet::*;
use serde_json::{json, Value};

#[tokio::test]
async fn old_wake_receipt_does_not_acknowledge_a_new_ready_generation() {
    let root = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(root.path().join("config"));
    let state = crate::new_state(root.path().join("work"), root.path().join("data"), None)
        .await
        .unwrap();
    let index = ExecutionIndex {
        id: "brain-wake-race".into(),
        node_id: "wake-node".into(),
        kind: ExecutionKind::Brain,
        status: ExecutionStatus::Idle,
        created_at: 1,
    };
    state.fleet.put_index(&index).await.unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    state
        .hub
        .attach(
            NodeRegistration {
                id: index.node_id.clone(),
                name: "wake test".into(),
                version: "test".into(),
                protocol_version: PROTOCOL_VERSION,
                maintenance_agent_id: "maintenance-wake-node".into(),
                kinds: vec![ExecutionKind::Brain],
            },
            NodeSnapshot {
                generation: "connection".into(),
                sequence: 1,
                cpu_capacity: 1.0,
                pending_runs: 0,
                active_agent_loops: 0,
                active_runs: 0,
                max_runs: 4,
                queue_order: Default::default(),
                ready: true,
                resource_error: None,
            },
            tx,
        )
        .await
        .unwrap();
    assert!(
        state
            .hub
            .mark_index_synced(&index.node_id, "connection")
            .await
    );
    let node_state = state.clone();
    let node_id = index.node_id.clone();
    let fake_node = tokio::spawn(async move {
        let mut reads = 0;
        loop {
            let SocketCommand::Frame(frame) = rx.recv().await.unwrap() else {
                panic!("unexpected socket close");
            };
            let ServerFrame::Call {
                request_id,
                operation,
            } = *frame;
            let NodeOperation::Brain { action, input, .. } = operation else {
                panic!("expected brain RPC");
            };
            let reply = match action.as_str() {
                "snapshot" => {
                    reads += 1;
                    // The previous wake already admitted a decision. Its fast
                    // child finishes before an extra post-wake snapshot read.
                    let (phase, generation) = if reads == 1 {
                        ("deciding", 1)
                    } else {
                        ("ready", 4)
                    };
                    json!({"schema_version":3,"run":{"run_id":"brain-wake-race",
                        "phase":phase,"generation":generation,"round":1,"last_event_seq":7,
                        "created_at":1,"updated_at":2,"error":null},"operations":[]})
                }
                "scheduler_wake_ack" => json!({"acknowledged":input["generation"]}),
                _ => panic!("unexpected action {action}"),
            };
            node_state
                .hub
                .resolve(&node_id, "connection", &request_id, RpcReply::ok(reply))
                .await;
            if action == "scheduler_wake_ack" {
                return input["generation"].clone();
            }
        }
    });
    crate::api::brain_runs::v3::delivery::deliver(
        &state,
        &index.node_id,
        &index.execution_ref(),
        "scheduler_wake",
        json!({"generation":0}),
    )
    .await
    .unwrap();
    assert_eq!(
        fake_node.await.unwrap(),
        Value::from(0),
        "a later ready generation still needs its own activation"
    );
}

use super::*;

async fn cancellation(
    mode: &str,
    active: u64,
    owned: u64,
    status: u16,
    creating: bool,
) -> (bool, Vec<String>) {
    let id = "brain-cancel-orphan";
    let (_directory, state, mut rx) = boot(id).await;
    state
        .fleet
        .put_index(&ExecutionIndex {
            id: "brain-never-accepted".into(),
            node_id: NODE.into(),
            kind: ExecutionKind::Brain,
            status: ExecutionStatus::Pending,
            created_at: 1,
        })
        .await
        .unwrap();
    let operation = json!({
        "operation_id":"orphan-operation", "run_id":id,"layer":1,"node_id":"nested",
        "attempt":1,"capability_id":"nested-plan","execution_kind":"brain",
        "execution_id":"brain-never-accepted", "status":if creating {"creating"} else {"running"},
        "source_sequence":null,"cancel_requested":true
    });
    let actions = Arc::new(Mutex::new(Vec::new()));
    let received = actions.clone();
    let owner = state.clone();
    let mode = mode.to_owned();
    let node_operation = operation.clone();
    let node = tokio::spawn(async move {
        while let Some(SocketCommand::Frame(frame)) = rx.recv().await {
            let ServerFrame::Call {
                request_id,
                operation,
            } = *frame;
            let reply = match operation {
                NodeOperation::Brain { action, input, .. } => {
                    received.lock().unwrap().push(action.clone());
                    match action.as_str() {
                        "snapshot" => {
                            let mut value = snapshot(id, "cancelled", 2, 1);
                            value["operations"] = json!([node_operation]);
                            RpcReply::ok(value)
                        }
                        "layered_terminal" => {
                            assert_eq!(input["execution_id"], "brain-never-accepted");
                            assert_eq!(input["status"], "cancelled");
                            assert_eq!(input["source_sequence"], 0);
                            RpcReply::ok(json!({"settled":true}))
                        }
                        "layered_cancel_ack" => RpcReply::ok(json!({"acknowledged":true})),
                        _ => panic!("unexpected action: {action}"),
                    }
                }
                NodeOperation::Command { execution, command } => {
                    assert_eq!(execution.id, "brain-never-accepted");
                    assert_eq!(command.action, "cancel");
                    received.lock().unwrap().push("child_cancel".into());
                    RpcReply::error(status, "execution not found")
                }
                NodeOperation::Admission { command } => {
                    assert_eq!(command, NodeAdmissionCommand::Status);
                    received.lock().unwrap().push("admission_status".into());
                    RpcReply::ok(json!({"mode":mode,"active_runs":active,"owned_processes":owned}))
                }
                _ => panic!("unexpected cancellation operation"),
            };
            owner
                .hub
                .resolve(NODE, CONNECTION, &request_id, reply)
                .await;
        }
    });
    let success = crate::api::brain_runs::v4::delivery::deliver(
        &state,
        NODE,
        &root(id),
        "layered_cancel",
        operation,
    )
    .await
    .is_ok();
    let actions = actions.lock().unwrap().clone();
    node.abort();
    let _ = node.await;
    (success, actions)
}

#[tokio::test]
async fn frozen_owner_missing_unadmitted_child_settles_before_cancellation_ack() {
    let (success, actions) = cancellation("frozen", 0, 0, 404, true).await;
    assert!(success);
    assert_eq!(
        actions,
        [
            "snapshot",
            "child_cancel",
            "admission_status",
            "layered_terminal",
            "layered_cancel_ack"
        ]
    );
}

#[tokio::test]
async fn missing_child_on_open_or_busy_owner_retains_cancellation_outbox() {
    for (mode, active, owned) in [("open", 0, 0), ("frozen", 1, 0), ("frozen", 0, 1)] {
        let (success, actions) = cancellation(mode, active, owned, 404, true).await;
        assert!(!success);
        assert!(!actions
            .iter()
            .any(|action| action == "layered_terminal" || action == "layered_cancel_ack"));
    }
}

#[tokio::test]
async fn offline_or_lost_running_child_cannot_be_declared_unadmitted() {
    for (status, creating) in [(503, true), (404, false)] {
        let (success, actions) = cancellation("frozen", 0, 0, status, creating).await;
        assert!(!success);
        assert!(!actions
            .iter()
            .any(|action| action == "layered_terminal" || action == "layered_cancel_ack"));
    }
}

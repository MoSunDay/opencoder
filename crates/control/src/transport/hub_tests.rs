use super::*;
use serde_json::json;
use std::sync::Arc;

fn registration() -> NodeRegistration {
    NodeRegistration {
        protocol_version: PROTOCOL_VERSION,
        id: "node-a".into(),
        name: "node-a".into(),
        version: "test".into(),
        maintenance_agent_id: "maintenance-node-a".into(),
        kinds: vec![ExecutionKind::Agent],
    }
}

fn snapshot(generation: &str, sequence: u64) -> NodeSnapshot {
    NodeSnapshot {
        pending_runs: 0,
        queue_order: Default::default(),
        generation: generation.into(),
        sequence,
        cpu_capacity: 2.0,
        active_agent_loops: 0,
        active_runs: 0,
        max_runs: 2,
        ready: true,
        resource_error: None,
    }
}

fn execution() -> ExecutionIndex {
    ExecutionIndex {
        id: "agent-late".into(),
        created_at: 7,
        kind: ExecutionKind::Agent,
        node_id: "node-a".into(),
        status: ExecutionStatus::Pending,
    }
}

fn create(index: &ExecutionIndex) -> NodeOperation {
    NodeOperation::Create {
        assignment: Assignment {
            runtime: None,
            codex: None,
            index: index.clone(),
            request: CreateExecution {
                id: index.id.clone(),
                kind: index.kind,
                target: None,
                input: serde_json::Value::Null,
                node_id: Some(index.node_id.clone()),
            },
            definition: None,
        },
    }
}

async fn call_request_id(rx: &mut mpsc::Receiver<SocketCommand>) -> String {
    let SocketCommand::Frame(frame) = rx.recv().await.expect("socket command") else {
        panic!("unexpected socket close")
    };
    let ServerFrame::Call { request_id, .. } = *frame;
    request_id
}

#[tokio::test]
async fn legacy_node_cannot_join_or_receive_harness_assignments() {
    let hub = Hub::new(vec![]);
    let (tx, mut rx) = mpsc::channel(1);
    let mut legacy = registration();
    legacy.protocol_version = 4;
    let error = hub
        .attach(legacy, snapshot("legacy", 1), tx)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("incompatible node protocol 4"));
    assert!(hub.views().await.is_empty());
    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn initial_report_gates_calls_and_scheduling_until_complete() {
    let hub = Hub::new(vec![]);
    let (tx, _rx) = mpsc::channel(1);
    hub.attach(registration(), snapshot("g1", 1), tx)
        .await
        .unwrap();
    let views = hub.views().await;
    assert!(!views[0].snapshot.as_ref().unwrap().ready);
    assert!(select_node(&views, ExecutionKind::Agent, None, now_ms()).is_none());
    let reply = hub
        .call(
            "node-a",
            NodeOperation::Maintenance {
                command: ExecutionCommand {
                    action: "status".into(),
                    input: serde_json::Value::Null,
                },
            },
        )
        .await;
    assert_eq!(reply.status, 503);

    assert!(hub.mark_index_synced("node-a", "g1").await);
    let views = hub.views().await;
    assert!(views[0].snapshot.as_ref().unwrap().ready);
    assert!(select_node(&views, ExecutionKind::Agent, None, now_ms()).is_some());
}

#[tokio::test]
async fn old_generation_cannot_touch_or_complete_new_connection() {
    let hub = Hub::new(vec![]);
    let (tx1, _rx1) = mpsc::channel(1);
    hub.attach(registration(), snapshot("g1", 1), tx1)
        .await
        .unwrap();
    hub.detach("node-a", "g1").await;
    let (tx2, _rx2) = mpsc::channel(1);
    hub.attach(registration(), snapshot("g2", 1), tx2)
        .await
        .unwrap();

    assert!(!hub.touch("node-a", "g1").await);
    assert!(!hub.mark_index_synced("node-a", "g1").await);
    hub.snapshot("node-a", "g2", snapshot("g1", 99)).await;
    let view = hub.views().await.remove(0);
    assert_eq!(view.snapshot.as_ref().unwrap().generation, "g2");
    assert!(!view.snapshot.unwrap().ready);
    assert!(hub.mark_index_synced("node-a", "g2").await);
}

#[tokio::test]
async fn late_failure_cannot_reject_an_inflight_create_or_leak_claims() {
    let hub = Arc::new(Hub::new(vec![]));
    let (tx, mut rx) = mpsc::channel(1);
    hub.attach(registration(), snapshot("g1", 1), tx)
        .await
        .unwrap();
    assert!(hub.mark_index_synced("node-a", "g1").await);
    let index = execution();
    hub.reserve(&index).await;
    hub.reserve(&index).await;
    assert_eq!(hub.views().await[0].reserved_loops, 2);

    let pending_hub = hub.clone();
    let expected = index.clone();
    let call = tokio::spawn(async move {
        pending_hub
            .call_with_timeout("node-a", create(&expected), Duration::from_millis(10))
            .await
    });
    let request_id = call_request_id(&mut rx).await;
    assert_eq!(call.await.unwrap().status, 504);

    let active_hub = hub.clone();
    let active_index = index.clone();
    let active = tokio::spawn(async move {
        active_hub
            .call_with_timeout("node-a", create(&active_index), Duration::from_secs(1))
            .await
    });
    let active_request = call_request_id(&mut rx).await;
    let failed = hub
        .resolve(
            "node-a",
            "g1",
            &request_id,
            RpcReply::error(400, "first attempt failed"),
        )
        .await
        .expect("timed-out create must retain settlement identity");
    assert_eq!(failed.execution.id, "agent-late");
    assert!(failed.late);
    assert!(!failed.reject_pending);
    assert_eq!(hub.views().await[0].reserved_loops, 1);

    let accepted = hub
        .resolve(
            "node-a",
            "g1",
            &active_request,
            RpcReply::ok(json!(failed.execution)),
        )
        .await
        .expect("active create must settle its claim");
    assert!(!accepted.late);
    assert_eq!(active.await.unwrap().status, 200);
    assert_eq!(hub.views().await[0].reserved_loops, 0);

    // A complete report is durable acceptance evidence. Even after the
    // successful request's own claim is gone, a conflicting peer reply cannot
    // reject the shared Pending index.
    hub.acknowledge_report("node-a", std::slice::from_ref(&accepted.execution))
        .await;
    hub.reserve(&accepted.execution).await;
    let conflict_hub = hub.clone();
    let conflict_index = accepted.execution.clone();
    let conflict = tokio::spawn(async move {
        conflict_hub
            .call_with_timeout("node-a", create(&conflict_index), Duration::from_secs(1))
            .await
    });
    let conflict_request = call_request_id(&mut rx).await;
    let rejected = hub
        .resolve(
            "node-a",
            "g1",
            &conflict_request,
            RpcReply::error(400, "conflicting attempt failed"),
        )
        .await
        .expect("failed create must settle its own claim");
    assert!(!rejected.reject_pending);
    assert_eq!(conflict.await.unwrap().status, 400);
    assert_eq!(hub.views().await[0].reserved_loops, 0);

    hub.detach("node-a", "g1").await;
    hub.reserve(&accepted.execution).await;
    assert_eq!(hub.views().await[0].reserved_loops, 1);
    assert_eq!(
        hub.call("node-a", create(&accepted.execution)).await.status,
        503
    );
    assert_eq!(hub.views().await[0].reserved_loops, 0);
}

#[tokio::test]
async fn post_operation_snapshot_precedes_capacity_claim_release() {
    let hub = Arc::new(Hub::new(vec![]));
    let (tx, mut rx) = mpsc::channel(1);
    hub.attach(registration(), snapshot("g1", 1), tx)
        .await
        .unwrap();
    assert!(hub.mark_index_synced("node-a", "g1").await);
    let index = execution();
    hub.reserve(&index).await;

    let call_hub = hub.clone();
    let call_index = index.clone();
    let call = tokio::spawn(async move {
        call_hub
            .call_with_timeout("node-a", create(&call_index), Duration::from_secs(1))
            .await
    });
    let request_id = call_request_id(&mut rx).await;
    let mut after_operation = snapshot("g1", 2);
    after_operation.active_agent_loops = 1;
    after_operation.active_runs = 1;
    hub.snapshot("node-a", "g1", after_operation).await;
    let before_reply = hub.views().await.remove(0);
    assert_eq!(before_reply.snapshot.unwrap().active_runs, 1);
    assert_eq!(before_reply.reserved_loops, 1);

    let settled = hub
        .resolve("node-a", "g1", &request_id, RpcReply::ok(json!(index)))
        .await
        .unwrap();
    assert!(!settled.late);
    assert_eq!(call.await.unwrap().status, 200);
    let after_reply = hub.views().await.remove(0);
    assert_eq!(after_reply.snapshot.unwrap().active_runs, 1);
    assert_eq!(after_reply.reserved_loops, 0);
}

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
            private_context: None,
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

#[test]
fn creation_rpc_uses_the_slow_admission_window() {
    assert_eq!(
        request_timeout(&create(&execution())),
        CREATE_REQUEST_TIMEOUT
    );
    assert_eq!(
        request_timeout(&NodeOperation::Maintenance {
            command: ExecutionCommand {
                action: "status".into(),
                input: serde_json::Value::Null,
            },
        }),
        DEFAULT_REQUEST_TIMEOUT
    );
}

#[test]
fn only_private_file_probes_use_the_digest_window() {
    for (private, expected) in [
        (false, DEFAULT_REQUEST_TIMEOUT),
        (true, CREATE_REQUEST_TIMEOUT),
    ] {
        let operation = NodeOperation::Brain {
            execution: execution().execution_ref(),
            action: "capability_probe".into(),
            input: serde_json::json!({"private_files":private}),
        };
        assert_eq!(request_timeout(&operation), expected);
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
async fn timed_out_call_releases_pending_and_claim_and_drops_late_reply() {
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
    // The timeout reclaimed the pending slot and one claim, so a late reply
    // for that request id is dropped instead of settling the dead call.
    assert!(hub
        .resolve(
            "node-a",
            "g1",
            &request_id,
            RpcReply::error(400, "late failure after timeout"),
        )
        .await
        .is_none());
    assert_eq!(hub.views().await[0].reserved_loops, 1);

    let active_hub = hub.clone();
    let active_index = index.clone();
    let active = tokio::spawn(async move {
        active_hub
            .call_with_timeout("node-a", create(&active_index), Duration::from_secs(1))
            .await
    });
    let active_request = call_request_id(&mut rx).await;
    let accepted = hub
        .resolve("node-a", "g1", &active_request, RpcReply::ok(json!(index)))
        .await
        .expect("active create must settle its claim");
    assert!(!accepted.late);
    assert_eq!(active.await.unwrap().status, 200);
    assert_eq!(hub.views().await[0].reserved_loops, 0);
    assert!(hub.state.lock().await.pending.is_empty());

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
async fn unanswered_timeouts_do_not_accumulate_pending_entries() {
    let hub = Arc::new(Hub::new(vec![]));
    let (tx, mut rx) = mpsc::channel(16);
    hub.attach(registration(), snapshot("g1", 1), tx)
        .await
        .unwrap();
    assert!(hub.mark_index_synced("node-a", "g1").await);

    let indices: Vec<_> = (0..8)
        .map(|n| ExecutionIndex {
            id: format!("agent-unanswered-{n}"),
            ..execution()
        })
        .collect();
    for index in &indices {
        hub.reserve(index).await;
    }
    assert_eq!(hub.views().await[0].reserved_loops, 8);

    // Keep one execution id back to prove a retry with the same id still
    // dispatches after every timeout reclaimed its slot.
    let retry_index = indices[0].clone();
    let mut calls = Vec::new();
    for index in indices {
        let call_hub = hub.clone();
        calls.push(tokio::spawn(async move {
            call_hub
                .call_with_timeout("node-a", create(&index), Duration::from_millis(10))
                .await
        }));
    }
    for _ in 0..8 {
        call_request_id(&mut rx).await;
    }
    for call in calls {
        assert_eq!(call.await.unwrap().status, 504);
    }
    assert!(hub.state.lock().await.pending.is_empty());
    assert_eq!(hub.views().await[0].reserved_loops, 0);

    // Reclaimed slots keep the hub dispatching: retrying with the same
    // execution id reaches the node and settles with a 200, no
    // "control channel busy".
    hub.reserve(&retry_index).await;
    let retry_hub = hub.clone();
    let retry_expected = retry_index.clone();
    let retry = tokio::spawn(async move {
        retry_hub
            .call_with_timeout("node-a", create(&retry_expected), Duration::from_secs(1))
            .await
    });
    let retry_request = call_request_id(&mut rx).await;
    let settled = hub
        .resolve(
            "node-a",
            "g1",
            &retry_request,
            RpcReply::ok(json!(retry_index)),
        )
        .await
        .expect("retry create must settle its claim");
    assert!(!settled.late);
    assert_eq!(retry.await.unwrap().status, 200);
    assert!(hub.state.lock().await.pending.is_empty());
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

#[tokio::test]
async fn delta_membership_preserves_unmentioned_acceptances_and_reservations() {
    let hub = Hub::new(vec![]);
    let a = ExecutionIndex {
        id: "agent-a".into(),
        created_at: 1,
        kind: ExecutionKind::Agent,
        node_id: "node-a".into(),
        status: ExecutionStatus::Pending,
    };
    let b = ExecutionIndex {
        id: "agent-b".into(),
        ..a.clone()
    };
    hub.acknowledge_report("node-a", &[a.clone(), b.clone()])
        .await;
    hub.reserve(&a).await;
    hub.reserve(&b).await;
    hub.acknowledge_report_changes("node-a", &[], true, std::slice::from_ref(&b.id))
        .await;
    let state = hub.state.lock().await;
    assert_eq!(
        state.accepted.get(&a.id).map(String::as_str),
        Some("node-a")
    );
    assert!(!state.accepted.contains_key(&b.id));
    assert!(
        state.reservations.contains_key(&a.id) && state.reservations.contains_key(&b.id),
        "inventory removal cannot release active capacity"
    );
}

#[tokio::test]
async fn unregister_sweeps_per_node_acceptances_and_reservations() {
    let fleet = opencoder_store::fleet::FleetStore::open_memory()
        .await
        .unwrap();
    let hub = Hub::new(vec![registration()]);
    let own = execution();
    let other = ExecutionIndex {
        id: "agent-kept".into(),
        node_id: "node-b".into(),
        ..own.clone()
    };
    hub.acknowledge_report("node-a", std::slice::from_ref(&own))
        .await;
    hub.acknowledge_report("node-b", std::slice::from_ref(&other))
        .await;
    hub.reserve(&own).await;
    hub.reserve(&other).await;
    let result = hub.unregister("node-a", &fleet).await.unwrap();
    assert!(matches!(result, UnregisterResult::Removed));
    let state = hub.state.lock().await;
    assert!(!state.nodes.contains_key("node-a"), "view removed");
    assert!(
        !state.accepted.values().any(|node| node == "node-a"),
        "removed node must not keep accepted markers"
    );
    assert!(
        !state.reservations.values().any(|r| r.node_id == "node-a"),
        "removed node must not keep reservations"
    );
    // An unrelated node's bookkeeping survives the sweep.
    assert_eq!(
        state.accepted.get(&other.id).map(String::as_str),
        Some("node-b")
    );
    assert!(state.reservations.contains_key(&other.id));
}

#[tokio::test]
async fn reconcile_reservations_releases_only_non_pending_entries() {
    let hub = Hub::new(vec![]);
    let (tx, _rx) = mpsc::channel(1);
    hub.attach(registration(), snapshot("g1", 1), tx)
        .await
        .unwrap();
    let live = execution();
    let mut orphan = execution();
    orphan.id = "agent-gone".into();
    hub.reserve(&live).await;
    hub.reserve(&orphan).await;
    hub.reserve(&orphan).await;
    assert_eq!(hub.views().await[0].reserved_loops, 3);

    let pending = std::iter::once(live.id.clone()).collect();

    // A fresh orphan is still inside the admission grace window: its
    // assignment write may lag the persisted Pending set, so it must NOT be
    // released yet.
    let now = opencoder_core::message::now_ms();
    assert_eq!(hub.reconcile_reservations(&pending, now).await, 0);
    assert_eq!(hub.views().await[0].reserved_loops, 3, "grace window holds");

    // Past the grace window the orphan is provably stale and is released.
    let late = now + 151_000;
    assert_eq!(hub.reconcile_reservations(&pending, late).await, 1);
    let view = hub.views().await.remove(0);
    assert_eq!(
        view.reserved_loops, 1,
        "orphan claims return, live claim stays"
    );
    {
        let state = hub.state.lock().await;
        assert!(state.reservations.contains_key(&live.id));
        assert!(!state.reservations.contains_key(&orphan.id));
    }

    // Idempotent: a clean sweep has nothing left to release.
    assert_eq!(hub.reconcile_reservations(&pending, late).await, 0);
    assert_eq!(hub.views().await[0].reserved_loops, 1);
}

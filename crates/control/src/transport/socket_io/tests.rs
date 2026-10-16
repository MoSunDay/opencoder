use super::*;
use opencoder_core::fleet::NodeSnapshot;

fn snapshot(sequence: u64) -> NodeFrame {
    NodeFrame::Snapshot {
        snapshot: NodeSnapshot {
            generation: "queue-generation".into(),
            sequence,
            cpu_capacity: 1.0,
            active_agent_loops: 0,
            active_runs: 0,
            pending_runs: 0,
            max_runs: 1,
            queue_order: Default::default(),
            ready: true,
            resource_error: None,
        },
    }
}

#[tokio::test]
async fn full_incoming_queue_preserves_order_without_blocking_handoff_write() {
    let root = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(root.path().join("config"));
    let state = crate::new_state(root.path().join("work"), root.path().join("data"), None)
        .await
        .unwrap();
    let (writer, mut written) = futures::channel::mpsc::channel(1);
    let (mut wire, reader) = futures::channel::mpsc::unbounded();
    let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = reads.clone();
    let reader = reader.inspect(move |_| {
        counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    let (_commands, commands) = mpsc::channel(1);
    let (acks, acknowledgements) = mpsc::channel(1);
    let (incoming, mut frames) = mpsc::channel(1);
    let capacity = incoming.clone();
    let task = tokio::spawn(
        Transport {
            writer,
            reader,
            commands,
            acknowledgements,
            incoming,
        }
        .run(state, "queue-node".into(), "queue-generation".into()),
    );
    for sequence in 1..=3 {
        wire.send(Ok(Message::Text(
            serde_json::to_string(&snapshot(sequence)).unwrap(),
        )))
        .await
        .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(1), async {
        while capacity.capacity() != 0 || reads.load(std::sync::atomic::Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
        // The third send has completed; the reader has consumed the second
        // frame and is now backpressured with one application frame pending.
        let ack = acknowledge(&acks, Message::Text("committed-handoff".into()));
        let (result, message) = tokio::join!(ack, written.next());
        result.unwrap();
        assert!(matches!(message, Some(Message::Text(text)) if text == "committed-handoff"));
        for expected in 1..=3 {
            let NodeFrame::Snapshot { snapshot } = frames.recv().await.unwrap() else {
                panic!("application frame changed type")
            };
            assert_eq!(snapshot.sequence, expected);
        }
    })
    .await
    .expect("a full incoming queue blocked the transport's handoff write");
    wire.send(Ok(Message::Close(None))).await.unwrap();
    task.await.unwrap().unwrap();
}

async fn idle_transport_is_bounded(full: bool) {
    let root = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(root.path().join("config"));
    let state = crate::new_state(root.path().join("work"), root.path().join("data"), None)
        .await
        .unwrap();
    let (writer, written) = futures::channel::mpsc::unbounded();
    let (mut wire, reader) = futures::channel::mpsc::unbounded();
    let (_commands, commands) = mpsc::channel(1);
    let (acks, acknowledgements) = mpsc::channel(1);
    let (incoming, _frames) = mpsc::channel(1);
    let capacity = incoming.clone();
    let task = tokio::spawn(
        Transport {
            writer,
            reader,
            commands,
            acknowledgements,
            incoming,
        }
        .run_with_budget(
            state,
            "queue-node".into(),
            "queue-generation".into(),
            Duration::from_millis(80),
        ),
    );
    if full {
        for sequence in 1..=2 {
            wire.send(Ok(Message::Text(
                serde_json::to_string(&snapshot(sequence)).unwrap(),
            )))
            .await
            .unwrap();
        }
        while capacity.capacity() != 0 {
            tokio::task::yield_now().await;
        }
    }
    let drain = tokio::spawn(written.for_each(|_| async {}));
    let sender = tokio::spawn(async move {
        loop {
            if acknowledge(&acks, Message::Text("server-traffic".into()))
                .await
                .is_err()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });
    let mut task = task;
    let result = tokio::time::timeout(Duration::from_millis(300), &mut task).await;
    if result.is_err() {
        task.abort();
    }
    sender.abort();
    drain.abort();
    let error = result
        .expect("server writes or a full queue postponed input liveness indefinitely")
        .unwrap()
        .expect_err("an input-stalled connection must stop");
    assert!(error.to_string().contains("input stalled"));
}

#[tokio::test]
async fn server_writes_do_not_postpone_input_liveness() {
    idle_transport_is_bounded(false).await;
}

#[tokio::test]
async fn full_input_queue_does_not_disable_input_liveness() {
    idle_transport_is_bounded(true).await;
}

#[tokio::test]
async fn incoming_heartbeats_renew_input_liveness() {
    let root = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(root.path().join("config"));
    let state = crate::new_state(root.path().join("work"), root.path().join("data"), None)
        .await
        .unwrap();
    let (writer, _written) = futures::channel::mpsc::unbounded();
    let (mut wire, reader) = futures::channel::mpsc::unbounded();
    let (_commands, commands) = mpsc::channel(1);
    let (_acks, acknowledgements) = mpsc::channel(1);
    let (incoming, _frames) = mpsc::channel(1);
    let task = tokio::spawn(
        Transport {
            writer,
            reader,
            commands,
            acknowledgements,
            incoming,
        }
        .run_with_budget(
            state,
            "queue-node".into(),
            "queue-generation".into(),
            Duration::from_millis(100),
        ),
    );
    for _ in 0..6 {
        wire.send(Ok(Message::Ping(vec![1]))).await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!task.is_finished(), "received input did not renew liveness");
    }
    wire.send(Ok(Message::Close(None))).await.unwrap();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn stalled_socket_detaches_and_same_generation_reconnect_preserves_owner() {
    use opencoder_core::fleet::*;
    use serde_json::json;
    use tokio_tungstenite::tungstenite::Message as Wire;

    let root = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(root.path().join("config"));
    let state = crate::new_state(root.path().join("work"), root.path().join("data"), None)
        .await
        .unwrap();
    let node = "node-input-liveness";
    let generation = "input-liveness-generation";
    let owned = ExecutionIndex {
        id: "dag-input-liveness-owned".into(),
        node_id: node.into(),
        kind: ExecutionKind::Dag,
        status: ExecutionStatus::Running,
        created_at: 1,
    };
    state.fleet.put_index(&owned).await.unwrap();
    let mut hello = snapshot(1);
    let NodeFrame::Snapshot { snapshot: initial } = &mut hello else {
        unreachable!()
    };
    initial.generation = generation.into();
    let hello = NodeFrame::Hello {
        registration: NodeRegistration {
            id: node.into(),
            name: "input liveness".into(),
            version: "test".into(),
            protocol_version: PROTOCOL_VERSION,
            maintenance_agent_id: "maintenance-input-liveness".into(),
            kinds: vec![ExecutionKind::Dag],
        },
        snapshot: initial.clone(),
    };
    let app = axum::Router::new()
        .route(
            "/channel",
            axum::routing::get(crate::transport::socket::upgrade),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "ws://{}/channel?node_id={node}",
        listener.local_addr().unwrap()
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (mut old, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .unwrap();
    old.send(Wire::Text(serde_json::to_string(&hello).unwrap()))
        .await
        .unwrap();
    let Wire::Text(first) = old.next().await.unwrap().unwrap() else {
        panic!("initial admission frame missing")
    };
    let ServerFrame::Call { request_id, .. } = serde_json::from_str(&first).unwrap();
    old.send(Wire::Text(
        serde_json::to_string(&NodeFrame::Reply {
            request_id,
            reply: RpcReply::ok(json!({"mode":"open"})),
        })
        .unwrap(),
    ))
    .await
    .unwrap();
    assert!(state.hub.mark_index_synced(node, generation).await);
    let owner = state.clone();
    let calls = tokio::spawn(async move {
        let mut jobs = tokio::task::JoinSet::new();
        loop {
            let owner = owner.clone();
            jobs.spawn(async move {
                owner
                    .hub
                    .call(
                        node,
                        NodeOperation::Maintenance {
                            command: ExecutionCommand {
                                action: "status".into(),
                                input: serde_json::Value::Null,
                            },
                        },
                    )
                    .await
            });
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    // The client keeps reading outgoing calls but provides no further input.
    tokio::time::timeout(Duration::from_millis(STALE_MS as u64 + 3000), async {
        loop {
            match old.next().await {
                Some(Ok(Wire::Close(_))) | Some(Err(_)) | None => break,
                _ => {}
            }
        }
    })
    .await
    .expect("outgoing traffic retained a dead input connection");
    calls.abort();
    let (mut fresh, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .unwrap();
    fresh
        .send(Wire::Text(serde_json::to_string(&hello).unwrap()))
        .await
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(2), fresh.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(response, Wire::Text(_)),
        "same generation was rejected after detach"
    );
    assert!(state.hub.mark_index_synced(node, generation).await);
    let retained = state.fleet.index(&owned.id).await.unwrap().unwrap();
    assert_eq!(retained, owned);
    fresh.close(None).await.unwrap();
    server.abort();
}

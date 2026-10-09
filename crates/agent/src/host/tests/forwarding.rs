use super::*;

#[tokio::test]
async fn runtime_schema_rejection_remains_definitive_without_changing_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let host = Host::open(&dir.path().join("host"), "node".into(), "test".into(), 1)
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new().route(
        "/rpc",
        axum::routing::post(|| async {
            (
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                "Failed to deserialize the JSON body: unknown field",
            )
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    host.store
        .register_runtime(&opencoder_store::fleet::handoff::RuntimeRecord {
            id: "original".into(),
            release_id: "original".into(),
            mode: "staged".into(),
            config: json!({"endpoint":endpoint,"data_dir":dir.path().join("runtime"),
                "unit":"opencoder-runtime-original.service"}),
        })
        .await
        .unwrap();
    host.store.activate_runtime("original").await.unwrap();
    let reply = host.handle(create(&host, "agent-rejected-schema")).await;
    assert_eq!(
        reply.status, 422,
        "a definitive rejection must not become a retryable 503"
    );
    assert!(reply.body["error"]
        .as_str()
        .unwrap()
        .contains("not accepted"));
    assert_eq!(
        host.store
            .owner("agent-rejected-schema")
            .await
            .unwrap()
            .unwrap()
            .runtime_id,
        "original"
    );
    let lock = tokio::time::timeout(
        Duration::from_secs(1),
        host.store.request_lock("runtime-use", "original"),
    )
    .await
    .expect("rejected request retained its runtime lock")
    .unwrap();
    drop(lock);
    server.abort();
}

async fn assert_stalled_forward_releases_its_lock(creation: bool) {
    let dir = tempfile::tempdir().unwrap();
    let host = Host::open(
        &dir.path().join("host"),
        "node".into(),
        "test-token".into(),
        1,
    )
    .await
    .unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let seen = requests.clone();
    let started = entered.clone();
    let app = axum::Router::new().route(
        "/rpc",
        axum::routing::post(move |axum::Json(operation): axum::Json<NodeOperation>| {
            let seen = seen.clone();
            let started = started.clone();
            async move {
                let first = {
                    let mut requests = seen.lock().unwrap();
                    requests.push(serde_json::to_value(&operation).unwrap());
                    requests.len() == 1
                };
                if first {
                    started.notify_one();
                    std::future::pending::<()>().await;
                }
                let body = match operation {
                    NodeOperation::Create { assignment } => json!(assignment.index),
                    NodeOperation::Inspect { execution } => json!({"id":execution.id}),
                    _ => panic!("unexpected operation"),
                };
                axum::Json(RpcReply::ok(body))
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    host.store
        .register_runtime(&opencoder_store::fleet::handoff::RuntimeRecord {
            id: "slow-runtime".into(),
            release_id: "slow-release".into(),
            mode: "staged".into(),
            config: json!({"endpoint":endpoint,"data_dir":dir.path().join("runtime"),
                "unit":"opencoder-runtime-slow.service"}),
        })
        .await
        .unwrap();
    host.store.activate_runtime("slow-runtime").await.unwrap();
    let operation = if creation {
        create(&host, "agent-slow-forward")
    } else {
        host.store
            .assign_runtime("agent-slow-forward", None)
            .await
            .unwrap();
        NodeOperation::Inspect {
            execution: ExecutionRef {
                id: "agent-slow-forward".into(),
                kind: ExecutionKind::Agent,
            },
        }
    };
    let waiting = host.clone();
    let original = operation.clone();
    let call = tokio::spawn(async move { waiting.handle(original).await });
    entered.notified().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(if creation { 46 } else { 11 })).await;
    let reply = call.await.unwrap();
    tokio::time::resume();
    assert_eq!(reply.status, 504, "{reply:?}");
    assert!(reply.body["error"]
        .as_str()
        .unwrap()
        .contains("runtime request timed out"));
    let lock = tokio::time::timeout(
        Duration::from_secs(1),
        host.store.request_lock("runtime-use", "slow-runtime"),
    )
    .await
    .expect("timed-out forwarding retained the shared runtime lock")
    .unwrap();
    drop(lock);
    assert_eq!(
        host.store
            .owner("agent-slow-forward")
            .await
            .unwrap()
            .unwrap()
            .runtime_id,
        "slow-runtime"
    );
    assert_eq!(host.handle(operation).await.status, 200);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1], "retry changed the frozen request");
    server.abort();
}

#[tokio::test]
async fn stalled_runtime_create_releases_its_lock_and_preserves_retry_ownership() {
    assert_stalled_forward_releases_its_lock(true).await;
}

#[tokio::test]
async fn stalled_runtime_inspection_releases_its_lock_before_server_read_timeout() {
    assert_stalled_forward_releases_its_lock(false).await;
}

#[tokio::test]
async fn stalled_legacy_creates_are_bounded_and_cannot_fill_the_host_channel() {
    let dir = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(dir.path().join("home"));
    let host = Host::open(
        &dir.path().join("host"),
        "bounded".into(),
        "test".into(),
        20,
    )
    .await
    .unwrap();
    let (entered, mut requests) = tokio::sync::mpsc::channel(8);
    let old_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let old_endpoint = format!("http://{}", old_listener.local_addr().unwrap());
    let slow = axum::Router::new().route(
        "/rpc",
        axum::routing::post(move |axum::Json(operation): axum::Json<NodeOperation>| {
            let entered = entered.clone();
            async move {
                if matches!(&operation, NodeOperation::Create { assignment }
                    if !opencoder_worker::requires_agent_pool(assignment))
                {
                    return axum::Json(RpcReply::ok(json!({"accepted":true})));
                }
                entered.send(()).await.unwrap();
                std::future::pending::<axum::Json<RpcReply>>().await
            }
        }),
    );
    let slow_server = tokio::spawn(async move { axum::serve(old_listener, slow).await.unwrap() });
    let new_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let new_endpoint = format!("http://{}", new_listener.local_addr().unwrap());
    let fast = axum::Router::new().route(
        "/rpc",
        axum::routing::post(|| async { axum::Json(RpcReply::ok(json!({"accepted":true}))) }),
    );
    let fast_server = tokio::spawn(async move { axum::serve(new_listener, fast).await.unwrap() });
    for (id, endpoint) in [("legacy", old_endpoint), ("current", new_endpoint)] {
        host.store
            .register_runtime(&opencoder_store::fleet::handoff::RuntimeRecord {
                id: id.into(),
                release_id: id.into(),
                mode: "staged".into(),
                config: json!({"endpoint":endpoint,"data_dir":dir.path().join(id),
                "unit":format!("opencoder-runtime-{id}.service")}),
            })
            .await
            .unwrap();
    }
    let mut pending = Vec::new();
    for index in 0..4 {
        let owner = host.clone();
        let operation = create(&host, &format!("agent-held-{index}"));
        pending.push(tokio::spawn(async move {
            owner.call_runtime("legacy", &operation).await.unwrap()
        }));
        tokio::time::timeout(Duration::from_secs(2), requests.recv())
            .await
            .unwrap()
            .unwrap();
    }
    // The old version never acknowledges Create. Repeated submissions must
    // return promptly rather than occupy all 128 fleet RPC permits.
    for index in 0..128 {
        let operation = create(&host, &format!("agent-extra-{index}"));
        let reply = tokio::time::timeout(
            Duration::from_secs(1),
            host.call_runtime("legacy", &operation),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(reply.status, 503);
    }
    assert_eq!(
        host.call_runtime("legacy", &create(&host, "agent-held-0"))
            .await
            .unwrap()
            .status,
        503
    );
    let mut native = create(&host, "dag-independent-native");
    if let NodeOperation::Create { assignment } = &mut native {
        assignment.index.kind = ExecutionKind::Dag;
        assignment.request.kind = ExecutionKind::Dag;
        assignment.request.target = None;
        assignment.request.input = json!({});
        assignment.definition = Some(json!({"name":"independent","steps":[{
            "name":"run","kind":{"type":"binary","resource":"quick"}}]}));
    }
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), host.call_runtime("legacy", &native))
            .await
            .unwrap()
            .unwrap()
            .status,
        200,
        "pure binary admission waited for unrelated cold resource copies"
    );
    assert_eq!(
        host.call_runtime("current", &create(&host, "agent-current"))
            .await
            .unwrap()
            .status,
        200
    );
    assert_eq!(
        host.call_runtime("current", &create(&host, "agent-current"))
            .await
            .unwrap()
            .status,
        200
    );
    let inspect = NodeOperation::Inspect {
        execution: ExecutionRef {
            id: "agent-held-0".into(),
            kind: ExecutionKind::Agent,
        },
    };
    let owner = host.clone();
    let query = tokio::spawn(async move { owner.call_runtime("legacy", &inspect).await.unwrap() });
    tokio::time::timeout(Duration::from_secs(2), requests.recv())
        .await
        .unwrap()
        .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(16)).await;
    assert_eq!(
        query.await.unwrap().status,
        504,
        "read RPC must have a bounded lifetime"
    );
    assert!(
        pending.iter().all(|task| !task.is_finished()),
        "cold creation retains its larger budget"
    );
    tokio::time::advance(Duration::from_secs(45)).await;
    for task in pending {
        assert_eq!(task.await.unwrap().status, 504);
    }
    let retry = create(&host, "agent-held-0");
    assert!(
        host.creations.begin("legacy", &retry).is_ok(),
        "timeout leaked creation capacity"
    );
    slow_server.abort();
    fast_server.abort();
}

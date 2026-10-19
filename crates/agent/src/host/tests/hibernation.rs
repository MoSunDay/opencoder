use super::*;

#[tokio::test]
async fn sleeping_inventory_does_not_probe_a_reused_endpoint_or_clear_its_marker() {
    let root = tempfile::tempdir().unwrap();
    let host = Host::open(&root.path().join("host"), "node".into(), "test".into(), 2)
        .await
        .unwrap();
    let model = Arc::new(MockChatClient::new());
    let (asleep_worker, asleep_http) = runtime(&host, root.path(), "asleep", model.clone()).await;
    host.store.activate_runtime("asleep").await.unwrap();
    let (active, active_http) = runtime(&host, root.path(), "active", model).await;
    host.store.activate_runtime("active").await.unwrap();
    let asleep = host.runtime("asleep").await.unwrap();
    let mut inventory = host.inventory(&asleep, false).await.unwrap();
    inventory.indexes.push(ExecutionIndex {
        id: "dag-historical".into(),
        node_id: host.registration.id.clone(),
        kind: ExecutionKind::Dag,
        status: ExecutionStatus::Done,
        created_at: 1,
    });
    let saved = serde_json::to_value(inventory).unwrap();
    host.store
        .put_definition("runtime_sleep", "asleep", &saved)
        .await
        .unwrap();
    asleep_http.abort();
    let _ = asleep_http.await;
    let endpoint = asleep.config["endpoint"].as_str().unwrap();
    let listener = tokio::net::TcpListener::bind(endpoint.trim_start_matches("http://"))
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    // The historical port now belongs to another service. Its response must
    // not be queried to rediscover an explicitly sleeping Runtime.
    let app = axum::Router::new().route(
        "/inventory",
        axum::routing::get(move || {
            let observed = observed.clone();
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "different service",
                )
            }
        }),
    );
    let unrelated = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let indexes = host.sync_inventory().await.unwrap();
    assert!(indexes
        .iter()
        .any(|index| index.id == "dag-historical" && index.status == ExecutionStatus::Done));
    assert_eq!(
        host.store
            .owner("dag-historical")
            .await
            .unwrap()
            .unwrap()
            .runtime_id,
        "asleep"
    );
    assert_eq!(
        host.store
            .definition("runtime_sleep", "asleep")
            .await
            .unwrap(),
        Some(saved)
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "sleeping endpoints must never be probed"
    );
    assert!(host.snapshot().ready);
    unrelated.abort();
    active_http.abort();
    asleep_worker.shutdown().await.unwrap();
    active.shutdown().await.unwrap();
}

#[tokio::test]
async fn unavailable_live_inventory_still_fails_readiness_without_a_sleep_marker() {
    let root = tempfile::tempdir().unwrap();
    let host = Host::open(&root.path().join("host"), "node".into(), "test".into(), 2)
        .await
        .unwrap();
    let (worker, http) = runtime(
        &host,
        root.path(),
        "active",
        Arc::new(MockChatClient::new()),
    )
    .await;
    host.store.activate_runtime("active").await.unwrap();
    let saved =
        serde_json::to_value(super::super::runtime::inventory_of(&worker).await.unwrap()).unwrap();
    http.abort();
    let _ = http.await;
    assert!(host.indexes().await.is_err());
    assert!(!host.snapshot().ready);
    assert!(host.snapshot().resource_error.is_some());
    assert!(host
        .store
        .definition("runtime_sleep", "active")
        .await
        .unwrap()
        .is_none());
    host.store
        .put_definition("runtime_sleep", "active", &saved)
        .await
        .unwrap();
    let error = host.indexes().await.unwrap_err();
    assert!(error.to_string().contains("requires a retired runtime"));
    assert!(
        !host.snapshot().ready,
        "an active Runtime cannot use final history"
    );
    assert_eq!(
        host.store
            .definition("runtime_sleep", "active")
            .await
            .unwrap(),
        Some(saved)
    );
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn admission_changes_leave_hibernated_runtimes_stopped() {
    let root = tempfile::tempdir().unwrap();
    let host = Host::open(
        &root.path().join("host"),
        "node".into(),
        "test-token".into(),
        1,
    )
    .await
    .unwrap();
    let model = Arc::new(MockChatClient::new());
    let (_, asleep_http) = runtime(&host, root.path(), "asleep", model.clone()).await;
    host.store.activate_runtime("asleep").await.unwrap();
    let (active, _active_http) = runtime(&host, root.path(), "active", model).await;
    host.store.activate_runtime("active").await.unwrap();
    let asleep = host.runtime("asleep").await.unwrap();
    let saved = serde_json::to_value(host.inventory(&asleep, false).await.unwrap()).unwrap();
    host.store
        .put_definition("runtime_sleep", "asleep", &saved)
        .await
        .unwrap();
    asleep_http.abort();
    let _ = asleep_http.await;
    for command in [
        NodeAdmissionCommand::Freeze,
        NodeAdmissionCommand::Status,
        NodeAdmissionCommand::Reopen,
    ] {
        let reply = host.handle(NodeOperation::Admission { command }).await;
        assert_eq!(reply.status, 200, "{:?}", reply.body);
        assert_eq!(reply.body["active_runs"], 0);
        assert_eq!(
            host.store
                .definition("runtime_sleep", "asleep")
                .await
                .unwrap(),
            Some(saved.clone())
        );
        assert_eq!(
            active.admission_open(),
            command == NodeAdmissionCommand::Reopen
        );
    }
}

#[tokio::test]
async fn accessed_hibernated_runtime_inherits_current_admission_mode() {
    let root = tempfile::tempdir().unwrap();
    let host = Host::open(
        &root.path().join("host"),
        "node".into(),
        "test-token".into(),
        1,
    )
    .await
    .unwrap();
    let model = Arc::new(MockChatClient::new());
    let (retired, _retired_http) = runtime(&host, root.path(), "retired", model.clone()).await;
    host.store.activate_runtime("retired").await.unwrap();
    let (_, _active_http) = runtime(&host, root.path(), "active", model).await;
    host.store.activate_runtime("active").await.unwrap();
    let runtime = host.runtime("retired").await.unwrap();
    let saved = serde_json::to_value(host.inventory(&runtime, false).await.unwrap()).unwrap();
    let read = NodeOperation::Maintenance {
        command: ExecutionCommand {
            action: "status".into(),
            input: json!({}),
        },
    };
    for command in [NodeAdmissionCommand::Freeze, NodeAdmissionCommand::Reopen] {
        host.store
            .put_definition("runtime_sleep", "retired", &saved)
            .await
            .unwrap();
        assert_eq!(
            host.handle(NodeOperation::Admission { command })
                .await
                .status,
            200
        );
        let reply =
            tokio::time::timeout(Duration::from_secs(2), host.call_runtime("retired", &read))
                .await
                .expect("waking a Runtime must not deadlock admission")
                .unwrap();
        assert_eq!(reply.status, 200, "{:?}", reply.body);
        assert_eq!(
            retired.admission_open(),
            command == NodeAdmissionCommand::Reopen
        );
        assert_eq!(
            host.store
                .definition("runtime_sleep", "retired")
                .await
                .unwrap(),
            Some(serde_json::Value::Null)
        );
    }
}

#[tokio::test]
async fn retired_runtime_storage_error_does_not_block_active_runtime() {
    let root = tempfile::tempdir().unwrap();
    let host = Host::open(
        &root.path().join("host"),
        "node".into(),
        "test-token".into(),
        1,
    )
    .await
    .unwrap();
    let model = Arc::new(MockChatClient::new());
    let (_retired, retired_http) = runtime(&host, root.path(), "r-retired", model.clone()).await;
    host.store.activate_runtime("r-retired").await.unwrap();
    let (active, _active_http) = runtime(&host, root.path(), "r-active", model).await;
    host.store.activate_runtime("r-active").await.unwrap();

    let retired = host
        .store
        .runtimes()
        .await
        .unwrap()
        .into_iter()
        .find(|runtime| runtime.id == "r-retired")
        .unwrap();
    let mut saved = host.inventory(&retired, false).await.unwrap();
    saved.snapshot.ready = false;
    saved.snapshot.resource_error = Some("node storage low".into());
    host.store
        .put_definition(
            "runtime_sleep",
            "r-retired",
            &serde_json::to_value(saved).unwrap(),
        )
        .await
        .unwrap();
    retired_http.abort();
    let _ = retired_http.await;

    assert!(active.snapshot().ready);
    host.sync_inventory().await.unwrap();
    let snapshot = host.snapshot();
    assert!(snapshot.ready, "{snapshot:?}");
    assert_eq!(snapshot.resource_error, None);
}

#[tokio::test]
async fn busy_runtime_hibernation_releases_the_fleet_activation_lock() {
    let root = tempfile::tempdir().unwrap();
    let host = Host::open(
        &root.path().join("host"),
        "node".into(),
        "test-token".into(),
        1,
    )
    .await
    .unwrap();
    let reader = host
        .store
        .shared_request_lock("runtime-use", "retired-busy")
        .await
        .unwrap();
    let error = tokio::time::timeout(Duration::from_secs(2), host.hibernate("retired-busy"))
        .await
        .expect("collection must yield while a runtime still serves requests")
        .unwrap_err();
    assert!(error.to_string().contains("in-flight requests"));
    let activation = tokio::time::timeout(
        Duration::from_secs(1),
        host.store.request_lock("release", "activation"),
    )
    .await
    .expect("unrelated releases must remain activatable")
    .unwrap();
    assert!(host
        .store
        .definition("runtime_sleep", "retired-busy")
        .await
        .unwrap()
        .is_none());
    drop(activation);
    drop(reader);
}

#[tokio::test]
async fn recently_read_retired_runtime_is_not_collected_between_history_requests() {
    let root = tempfile::tempdir().unwrap();
    let host = Host::open(&root.path().join("host"), "node".into(), "test".into(), 2)
        .await
        .unwrap();
    let model = Arc::new(MockChatClient::new());
    let (retired, retired_http) =
        runtime(&host, root.path(), "history-reader", model.clone()).await;
    host.store.activate_runtime("history-reader").await.unwrap();
    let (active, active_http) = runtime(&host, root.path(), "current-reader", model).await;
    host.store.activate_runtime("current-reader").await.unwrap();
    host.store
        .put_definition("host", "current", &json!({"instance":host.instance}))
        .await
        .unwrap();
    let read = NodeOperation::Maintenance {
        command: ExecutionCommand {
            action: "status".into(),
            input: json!({}),
        },
    };
    assert_eq!(
        host.call_runtime("history-reader", &read)
            .await
            .unwrap()
            .status,
        200
    );
    // Keep an attempted collection from invoking systemctl in this test.
    // Recent reads must skip collection before waiting on this shared lock.
    let activation = host
        .store
        .request_lock("release", "activation")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), host.collect_retired())
        .await
        .expect("recent history reads must defer automatic hibernation")
        .unwrap();
    assert!(host
        .store
        .definition("runtime_sleep", "history-reader")
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        host.call_runtime("history-reader", &read)
            .await
            .unwrap()
            .status,
        200
    );
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::time::resume();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), host.collect_retired())
            .await
            .is_err(),
        "an idle Runtime must become eligible for collection again"
    );
    drop(activation);
    retired_http.abort();
    active_http.abort();
    retired.shutdown().await.unwrap();
    active.shutdown().await.unwrap();
}

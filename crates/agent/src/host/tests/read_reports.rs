use super::*;

#[tokio::test]
async fn queries_preserve_revision_but_mutations_and_wakeups_notify() {
    let dir = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(dir.path().join("home"));
    let host = Host::open(&dir.path().join("host"), "reads".into(), "test".into(), 4)
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let inventory = serde_json::to_value(super::super::config::Inventory {
        runtime_id: Some("reads".into()),
        build: serde_json::to_value(opencoder_core::version::build_info()).unwrap(),
        owned_processes: 0,
        registration: host.registration.clone(),
        snapshot: host.snapshot(),
        indexes: vec![],
        can_hibernate: true,
    })
    .unwrap();
    let report = inventory.clone();
    let app = axum::Router::new()
        .route(
            "/rpc",
            axum::routing::post(|| async { axum::Json(RpcReply::ok(json!({"status":"running"}))) }),
        )
        .route(
            "/inventory",
            axum::routing::get(move || {
                let report = report.clone();
                async move { axum::Json(report) }
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    host.store
        .register_runtime(&opencoder_store::fleet::handoff::RuntimeRecord {
            id: "reads".into(),
            release_id: "reads".into(),
            mode: "staged".into(),
            config: json!({"endpoint":endpoint,"data_dir":dir.path().join("runtime"),
            "unit":"opencoder-runtime-reads.service"}),
        })
        .await
        .unwrap();
    let mut revision = host.changes.subscribe();
    let execution = ExecutionRef {
        id: "agent-read".into(),
        kind: ExecutionKind::Agent,
    };
    for operation in [
        NodeOperation::Inspect {
            execution: execution.clone(),
        },
        NodeOperation::Events {
            execution: execution.clone(),
            after: 0,
        },
        NodeOperation::AcceptedRequest {
            execution: execution.clone(),
        },
    ] {
        assert_eq!(
            host.call_runtime("reads", &operation).await.unwrap().status,
            200
        );
        assert!(!revision.has_changed().unwrap());
    }
    assert_eq!(
        host.call_runtime(
            "reads",
            &NodeOperation::Command {
                execution: execution.clone(),
                command: ExecutionCommand {
                    action: "stop".into(),
                    input: json!({})
                },
            }
        )
        .await
        .unwrap()
        .status,
        200
    );
    assert!(revision.has_changed().unwrap());
    revision.borrow_and_update();
    // A successful query that discovers a formerly sleeping Runtime is a
    // real inventory change even though the operation itself is read-only.
    host.store
        .put_definition("runtime_sleep", "reads", &inventory)
        .await
        .unwrap();
    assert_eq!(
        host.call_runtime("reads", &NodeOperation::Inspect { execution })
            .await
            .unwrap()
            .status,
        200
    );
    assert!(revision.has_changed().unwrap());
    assert!(
        host.store
            .definition("runtime_sleep", "reads")
            .await
            .unwrap()
            .unwrap()
            .is_null()
    );
    server.abort();
}

#[tokio::test]
async fn incremental_indexes_are_independent_of_brain_but_legacy_collection_stays_serial() {
    let dir = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(dir.path().join("home"));
    let host = Arc::new(
        Host::open(&dir.path().join("host"), "reports".into(), "test".into(), 4)
            .await
            .unwrap(),
    );
    assert!(host.supports_index_deltas());
    let entered = Arc::new(tokio::sync::Semaphore::new(0));
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let inventory = serde_json::to_value(super::super::config::Inventory {
        runtime_id: Some("reports".into()),
        build: serde_json::to_value(opencoder_core::version::build_info()).unwrap(),
        owned_processes: 0,
        registration: host.registration.clone(),
        snapshot: host.snapshot(),
        indexes: vec![],
        can_hibernate: false,
    })
    .unwrap();
    let input = entered.clone();
    let gate = release.clone();
    let app = axum::Router::new()
        .route(
            "/inventory",
            axum::routing::get(move || {
                let body = inventory.clone();
                async move { axum::Json(body) }
            }),
        )
        .route(
            "/frames",
            axum::routing::get(move || {
                let input = input.clone();
                let gate = gate.clone();
                async move {
                    input.add_permits(1);
                    gate.acquire().await.unwrap().forget();
                    axum::Json(Vec::<NodeFrame>::new())
                }
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    host.store.register_runtime(&opencoder_store::fleet::handoff::RuntimeRecord {
        id: "reports".into(), release_id: "reports".into(), mode: "staged".into(),
        config: json!({"endpoint":endpoint,"data_dir":dir.path().join("runtime"),"unit":"opencoder-runtime-reports.service"}),
    }).await.unwrap();
    host.store.activate_runtime("reports").await.unwrap();
    let source = host.clone();
    let legacy = tokio::spawn(async move { source.report().await });
    tokio::time::timeout(Duration::from_secs(3), entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let source = host.clone();
    let mut indexes = tokio::spawn(async move { source.index_report().await });
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut indexes)
            .await
            .is_err(),
        "legacy full reports must retain their collection gate through Brain"
    );
    release.add_permits(1);
    let legacy = legacy.await.unwrap().unwrap();
    let indexed = indexes.await.unwrap().unwrap();
    assert!(legacy.snapshot.sequence < indexed.snapshot.sequence);
    let source = host.clone();
    let brain = tokio::spawn(async move { source.brain_frames().await });
    tokio::time::timeout(Duration::from_secs(3), entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let independent = tokio::time::timeout(Duration::from_secs(1), host.index_report())
        .await
        .unwrap()
        .unwrap();
    assert!(independent.brain.is_empty());
    assert!(independent.snapshot.sequence > indexed.snapshot.sequence);
    release.add_permits(1);
    brain.await.unwrap().unwrap();
    server.abort();
}

#[tokio::test]
async fn status_proves_only_this_hosts_enabled_server_report_acknowledgements() {
    let dir = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(dir.path().join("home"));
    let data = dir.path().join("host");
    let old = Host::open(&data, "node".into(), "test".into(), 4)
        .await
        .unwrap();
    let new = Host::open(&data, "node".into(), "test".into(), 4)
        .await
        .unwrap();
    old.store
        .put_definition("release_server", "r1", &json!({"id":"r1","enabled":true}))
        .await
        .unwrap();
    let ack = || NodeOperation::Maintenance {
        command: ExecutionCommand {
            action: "host_handoff_ready".into(),
            input: json!({"server":"r1"}),
        },
    };
    assert_eq!(old.handle(ack()).await.status, 200);
    let status = new.status().await.unwrap();
    assert_eq!(status["index_report_version"], "2");
    assert_eq!(
        status["reported_servers"],
        json!([]),
        "an older Host ACK cannot promote a new producer"
    );
    assert_eq!(new.handle(ack()).await.status, 200);
    assert_eq!(
        new.status().await.unwrap()["reported_servers"],
        json!(["r1"])
    );
    assert_eq!(old.status().await.unwrap()["reported_servers"], json!([]));
    old.store
        .put_definition("release_server", "r1", &json!({"id":"r1","enabled":false}))
        .await
        .unwrap();
    assert_eq!(new.status().await.unwrap()["reported_servers"], json!([]));
}

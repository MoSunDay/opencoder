use super::*;

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
    let _scope = opencoder_core::config::scoped_config_home(root.path().join("config"));
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

    wait(async || active.snapshot().ready).await;
    assert!(active.snapshot().ready, "{:?}", active.snapshot());
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

use super::*;

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

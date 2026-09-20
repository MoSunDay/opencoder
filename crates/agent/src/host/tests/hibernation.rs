use super::*;

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

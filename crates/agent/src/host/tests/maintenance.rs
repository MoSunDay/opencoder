use super::*;

#[tokio::test]
async fn deployment_http_requires_authentication_current_host_and_current_server() {
    let dir = tempfile::tempdir().unwrap();
    let host = Host::open(
        &dir.path().join("host"),
        "node".into(),
        "signal-fixture".into(),
        1,
    )
    .await
    .unwrap();
    host.store
        .put_definition("host", "current", &json!({"instance":host.instance}))
        .await
        .unwrap();
    std::fs::write(
        dir.path().join("host/deployment.json"),
        serde_json::to_vec(&json!({
        "state_dir":dir.path(),"unit_prefix":"opencoder-release-fixture"}))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("release-state.json"),
        br#"{"current":"r2"}"#,
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "http://{}/deployment-signal",
        listener.local_addr().unwrap()
    );
    let app = api::router(host.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    assert_eq!(
        client
            .post(&endpoint)
            .json(&json!({"action":"deploy","release_id":"r2"}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    for (body, expected) in [
        (json!({"action":"deploy","release_id":"r1"}), "superseded"),
        (
            json!({"action":"restart","release_id":"r2"}),
            "unknown release action",
        ),
    ] {
        let reply = client
            .post(&endpoint)
            .bearer_auth("signal-fixture")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(reply.status(), 409);
        assert!(reply.text().await.unwrap().contains(expected));
    }
    host.store
        .put_definition("host", "current", &json!({"instance":"replacement"}))
        .await
        .unwrap();
    let reply = client
        .post(&endpoint)
        .bearer_auth("signal-fixture")
        .json(&json!({"action":"deploy","release_id":"r2"}))
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status(), 409);
    assert!(reply.text().await.unwrap().contains("Host is not current"));
    task.abort();
}

#[tokio::test]
async fn host_scheduling_reports_workdir_unsupported_and_rejects_workdir_with_400() {
    let dir = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(dir.path().join("home"));
    let host = Host::open(
        &dir.path().join("host"),
        "node".into(),
        "test-token".into(),
        1,
    )
    .await
    .unwrap();
    let maintenance = |action: &str, input: serde_json::Value| NodeOperation::Maintenance {
        command: ExecutionCommand {
            action: action.into(),
            input,
        },
    };
    // The read endpoint exposes the capability flag so the UI can hide the
    // workdir input instead of submitting a value the host must reject.
    let reply = host
        .handle(maintenance("scheduling", serde_json::Value::Null))
        .await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body["queue_order"], "fifo");
    assert_eq!(reply.body["workdir"], serde_json::Value::Null);
    assert_eq!(reply.body["workdir_supported"], false);
    // Capability violations are 400 client errors, never 503 "host routing".
    let reply = host
        .handle(maintenance(
            "configure_scheduling",
            json!({"max_runs":4,"queue_order":"fifo","workdir":"/data/x"}),
        ))
        .await;
    assert_eq!(reply.status, 400);
    let message = reply.body["error"].as_str().unwrap();
    assert!(message.contains("multi-runtime hosts do not support a scheduling workdir"));
    assert!(!message.contains("host routing"));
    let reply = host
        .handle(maintenance(
            "configure_scheduling",
            json!({"max_runs":4,"queue_order":"lifo"}),
        ))
        .await;
    assert_eq!(reply.status, 400);
    assert_eq!(
        reply.body["error"],
        "multi-runtime hosts require FIFO ordering"
    );
}

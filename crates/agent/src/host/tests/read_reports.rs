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
    let app = axum::Router::new().route(
        "/rpc",
        axum::routing::post(|| async { axum::Json(RpcReply::ok(json!({"status":"running"}))) }),
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
        .put_definition("runtime_sleep", "reads", &json!({"sleeping":true}))
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
    assert!(host
        .store
        .definition("runtime_sleep", "reads")
        .await
        .unwrap()
        .unwrap()
        .is_null());
    server.abort();
}

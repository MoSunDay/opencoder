use super::*;

async fn endpoint(host: &Host, root: &std::path::Path, id: &str) -> tokio::task::JoinHandle<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let identity = id.to_owned();
    let app = axum::Router::new().route(
        "/rpc",
        axum::routing::post(move |axum::Json(operation): axum::Json<NodeOperation>| {
            let identity = identity.clone();
            async move {
                axum::Json(RpcReply::ok(
                    json!({"runtime":identity,"operation":operation}),
                ))
            }
        }),
    );
    host.store.register_runtime(&opencoder_store::fleet::handoff::RuntimeRecord {
        id:id.into(), release_id:id.into(), mode:"staged".into(),
        config:json!({"endpoint":address,"data_dir":root.join(id),"unit":format!("opencoder-runtime-{id}.service")}),
    }).await.unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() })
}

fn request(host: &Host, id: &str, kind: ExecutionKind, input: serde_json::Value) -> NodeOperation {
    let mut operation = create(host, id);
    if let NodeOperation::Create { assignment } = &mut operation {
        assignment.index.kind = kind;
        assignment.request.kind = kind;
        assignment.request.input = input;
    }
    operation
}

fn probe(id: &str, kind: ExecutionKind, input: serde_json::Value) -> NodeOperation {
    NodeOperation::Brain {
        execution: ExecutionRef {
            id: id.into(),
            kind,
        },
        action: "capability_probe".into(),
        input,
    }
}

#[tokio::test]
async fn children_probe_create_and_replay_follow_the_parent_after_runtime_switch() {
    let dir = tempfile::tempdir().unwrap();
    let host = Host::open(&dir.path().join("host"), "node".into(), "test".into(), 1)
        .await
        .unwrap();
    let old = endpoint(&host, dir.path(), "old").await;
    let new = endpoint(&host, dir.path(), "new").await;
    host.store.activate_runtime("old").await.unwrap();
    host.store
        .assign_runtime("brain-parent", None)
        .await
        .unwrap();
    host.store.activate_runtime("new").await.unwrap();
    for (index, kind, input) in [
        (
            0,
            ExecutionKind::Operator,
            json!({"brain_workspace":true,"brain_layered":{"run_id":"brain-parent"}}),
        ),
        (
            1,
            ExecutionKind::Brain,
            json!({"layered_request":{"parent":{"run_id":"brain-parent"}}}),
        ),
        (
            2,
            ExecutionKind::Agent,
            json!({"_brain":{"parent":{"run_id":"brain-parent"}}}),
        ),
        (
            3,
            ExecutionKind::Operator,
            json!({"brain_layered":{"run_id":"brain-parent"}}),
        ),
    ] {
        let id = format!("child-{index}");
        let reply = host.handle(probe(&id, kind, input.clone())).await;
        assert_eq!(reply.status, 200, "{reply:?}");
        assert_eq!(reply.body["runtime"], "old");
        assert!(
            host.store.owner(&id).await.unwrap().is_none(),
            "probe must not freeze ownership"
        );
        let operation = request(&host, &id, kind, input);
        assert_eq!(host.handle(operation.clone()).await.body["runtime"], "old");
        assert_eq!(
            host.store.owner(&id).await.unwrap().unwrap().runtime_id,
            "old"
        );
        assert_eq!(host.handle(operation).await.body["runtime"], "old");
        // Once accepted, even a probe with no parent hint uses the original owner.
        assert_eq!(
            host.handle(probe(&id, kind, json!({}))).await.body["runtime"],
            "old"
        );
    }
    for input in [
        json!({}),
        json!({"brain_layered":{"run_id":"brain-remote"}}),
    ] {
        let reply = host
            .handle(probe(
                "child-independent",
                ExecutionKind::Operator,
                input.clone(),
            ))
            .await;
        assert_eq!(reply.body["runtime"], "new");
        assert_eq!(
            host.handle(request(
                &host,
                "child-independent",
                ExecutionKind::Operator,
                input
            ))
            .await
            .body["runtime"],
            "new"
        );
    }
    old.abort();
    new.abort();
}

#[tokio::test]
async fn required_unknown_parent_never_falls_back_to_active_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let host = Host::open(&dir.path().join("host"), "node".into(), "test".into(), 1)
        .await
        .unwrap();
    let server = endpoint(&host, dir.path(), "active").await;
    host.store.activate_runtime("active").await.unwrap();
    for (kind, input) in [
        (
            ExecutionKind::Operator,
            json!({"brain_workspace":true,"brain_layered":{"run_id":"brain-missing"}}),
        ),
        (ExecutionKind::Operator, json!({"brain_workspace":true})),
        (
            ExecutionKind::Brain,
            json!({"layered_request":{"parent":{"run_id":"brain-missing"}}}),
        ),
    ] {
        for op in [
            probe("child-missing", kind, input.clone()),
            request(&host, "child-missing", kind, input),
        ] {
            let reply = host.handle(op).await;
            assert!(reply.status >= 400, "{reply:?}");
            assert!(reply.body["error"].as_str().unwrap().contains("parent"));
            assert!(host.store.owner("child-missing").await.unwrap().is_none());
        }
    }
    server.abort();
}

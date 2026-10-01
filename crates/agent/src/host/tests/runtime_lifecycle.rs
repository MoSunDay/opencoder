use super::*;

#[tokio::test]
async fn three_runtime_versions_keep_live_model_calls_and_global_fifo() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();
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
    let held = Arc::new(HeldModel {
        entered: AtomicUsize::new(0),
        release: Arc::new(tokio::sync::Notify::new()),
    });
    let fast = Arc::new(
        MockChatClient::new().with_default(vec![LlmEvent::Completed {
            text: "new release".into(),
            tool_calls: vec![],
            usage: None,
        }]),
    );
    let (old, old_http) = runtime(&host, dir.path(), "r1", held.clone()).await;
    let (new, new_http) = runtime(&host, dir.path(), "r2", fast.clone()).await;
    let (latest, latest_http) = runtime(&host, dir.path(), "r3", fast.clone()).await;
    host.store.activate_runtime("r1").await.unwrap();
    assert_eq!(host.handle(create(&host, "agent-long")).await.status, 200);
    wait(async || held.entered.load(Ordering::SeqCst) == 1).await;
    host.store.activate_runtime("r2").await.unwrap();
    assert_eq!(host.handle(create(&host, "agent-next")).await.status, 200);
    host.store.activate_runtime("r3").await.unwrap();
    assert_eq!(host.handle(create(&host, "agent-latest")).await.status, 200);
    assert_eq!(host.store.capacity().await.unwrap().running, 1);
    assert_eq!(host.store.capacity().await.unwrap().queued, 2);
    assert_eq!(held.entered.load(Ordering::SeqCst), 1);
    assert_eq!(fast.call_count(), 0);
    let replay = host.handle(create(&host, "agent-long")).await;
    assert_eq!(replay.status, 200);
    assert_eq!(
        host.store
            .owner("agent-long")
            .await
            .unwrap()
            .unwrap()
            .runtime_id,
        "r1"
    );
    assert!(!old.can_hibernate().await);
    held.release.notify_one();
    wait(async || {
        host.store.capacity().await.unwrap().queued == 0
            && host.store.capacity().await.unwrap().running == 0
    })
    .await;
    assert_eq!(held.entered.load(Ordering::SeqCst), 1);
    assert_eq!(fast.call_count(), 2);
    assert_eq!(
        host.store
            .owner("agent-next")
            .await
            .unwrap()
            .unwrap()
            .runtime_id,
        "r2"
    );
    let detail = host
        .handle(NodeOperation::Inspect {
            execution: ExecutionRef {
                id: "agent-long".into(),
                kind: ExecutionKind::Agent,
            },
        })
        .await;
    assert_eq!(detail.status, 200);
    assert_eq!(detail.body["execution"]["status"], "idle");
    old_http.abort();
    new_http.abort();
    latest_http.abort();
    for worker in [old, new, latest] {
        worker.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn host_dialogs_clear_deletes_live_runtime_and_trims_hibernated_inventory() {
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
    let fast = Arc::new(
        MockChatClient::new().with_default(vec![LlmEvent::Completed {
            text: "operator done".into(),
            tool_calls: vec![],
            usage: None,
        }]),
    );
    let (live, _live_http) = runtime(&host, dir.path(), "r-live", fast.clone()).await;
    host.store.activate_runtime("r-live").await.unwrap();

    // A retired-but-hibernated runtime is only represented by its saved
    // final inventory; it holds one droppable and one live operator row.
    host.store
        .register_runtime(&opencoder_store::fleet::handoff::RuntimeRecord {
            id: "r-sleep".into(),
            release_id: "r-sleep".into(),
            mode: "staged".into(),
            config: json!({"endpoint":"http://127.0.0.1:1","data_dir":dir.path().join("r-sleep"),"unit":"opencoder-runtime-r-sleep.service"}),
        })
        .await
        .unwrap();
    // Retire r-live's predecessor slot by promoting r-sleep then re-activating
    // the real one, leaving r-sleep retired (and hibernated) in the catalog.
    host.store.activate_runtime("r-sleep").await.unwrap();
    host.store.activate_runtime("r-live").await.unwrap();
    let saved = json!({
        "registration": {"id": host.registration.id},
        "snapshot": {"ready": false},
        "indexes": [
            {"id":"operator-done-1","created_at":1,"kind":"operator","node_id":host.registration.id,"status":"idle"},
            {"id":"agent-done-1","created_at":1,"kind":"agent","node_id":host.registration.id,"status":"idle"},
            {"id":"operator-live-1","created_at":2,"kind":"operator","node_id":host.registration.id,"status":"interrupted"}
        ]
    });
    host.store
        .put_definition("runtime_sleep", "r-sleep", &saved)
        .await
        .unwrap();

    // One live operator execution on the active runtime.
    let reply = host
        .handle(NodeOperation::Create {
            assignment: Assignment {
                private_context: None,
                runtime: None,
                codex: None,
                definition: None,
                request: CreateExecution {
                    id: "operator-done-2".into(),
                    kind: ExecutionKind::Operator,
                    target: None,
                    input: json!({"prompt":"hi"}),
                    node_id: Some(host.registration.id.clone()),
                },
                index: ExecutionIndex {
                    id: "operator-done-2".into(),
                    kind: ExecutionKind::Operator,
                    node_id: host.registration.id.clone(),
                    created_at: 1,
                    status: ExecutionStatus::Pending,
                },
            },
        })
        .await;
    assert_eq!(reply.status, 200, "{:?}", reply);
    let journal = dir
        .path()
        .join("r-live/operator/operator-done-2/execution.json");
    wait(async || journal.is_file()).await;
    wait(async || {
        let inspect = live
            .indexes()
            .await
            .unwrap()
            .into_iter()
            .find(|i| i.id == "operator-done-2")
            .map(|i| i.status);
        inspect == Some(ExecutionStatus::Idle)
    })
    .await;

    let maintenance = |input: serde_json::Value| NodeOperation::Maintenance {
        command: ExecutionCommand {
            action: "dialogs_clear".into(),
            input,
        },
    };
    let reply = host
        .handle(maintenance(json!({"sessions": ["operator-done-1", "operator-done-2", "operator-live-1", "operator-ghost"]})))
        .await;
    assert_eq!(reply.status, 200, "{:?}", reply);
    assert_eq!(reply.body["removed"], json!(2));
    assert_eq!(reply.body["forgotten"], json!(1));
    assert_eq!(reply.body["skipped"], json!(["operator-live-1"]));

    // The live runtime lost its session and journal record.
    assert!(!journal.exists());
    // The hibernated inventory kept the live row and dropped the droppable one.
    let kept = host
        .store
        .definition("runtime_sleep", "r-sleep")
        .await
        .unwrap()
        .unwrap();
    let ids: Vec<String> = kept["indexes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["id"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(
        ids,
        vec!["agent-done-1".to_string(), "operator-live-1".to_string()]
    );
    // A second lane clear is independent and can remove the hibernated Agent
    // row without touching the surviving Operator record.
    let reply = host
        .handle(maintenance(json!({
            "kind": "agent",
            "sessions": ["agent-done-1", "operator-live-1"]
        })))
        .await;
    assert_eq!(reply.status, 200, "{:?}", reply);
    assert_eq!(reply.body["kind"], json!("agent"));
    assert_eq!(reply.body["removed"], json!(1));
    assert_eq!(reply.body["skipped"], json!([]));
    let kept = host
        .store
        .definition("runtime_sleep", "r-sleep")
        .await
        .unwrap()
        .unwrap();
    let ids: Vec<String> = kept["indexes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["id"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(ids, vec!["operator-live-1".to_string()]);
    live.shutdown().await.unwrap();
}

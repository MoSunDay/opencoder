use super::*;

#[tokio::test]
async fn empty_outbox_releases_the_last_parsed_dispatch_without_another_frame_poll() {
    let home = tempfile::tempdir().unwrap();
    let _guard = opencoder_core::config::scoped_config_home(home.path().into());
    let root = tempfile::tempdir().unwrap();
    let workdir = root.path().join("work");
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(workdir.join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
    let worker = Worker::open(
        crate::WorkerOptions {
            name: "memory-test".into(),
            workdir,
            data_dir: root.path().join("node"),
            workflow_root: None,
            max_runs: Some(1),
            dag: false,
        },
        Some(Arc::new(opencoder_llm::MockChatClient::new())),
    )
    .await
    .unwrap();
    let operation: opencoder_core::brain::layered::LayeredOperation = serde_json::from_value(json!({
        "operation_id":"op","run_id":"brain-root","round":1,"activation":0,"layer":1,
        "node_id":"step","attempt":1,"capability_id":"cap","execution_kind":"agent",
        "execution_id":"agent-child","status":"creating","source_sequence":null,"cancel_requested":false
    })).unwrap();
    let record = serde_json::from_value(json!({"assignment":{
        "index":{"id":"brain-root","kind":"brain","created_at":1,"node_id":"node-test","status":"idle"},
        "request":{"id":"brain-root","kind":"brain","input":{"schema_version":7,"layered_request":{}}}},
        "annotations":{"layered_intent":{"generation":1,"layer":1,"operations":[operation],
            "assignments":[{"node_id":"step","capability_id":"cap","reason":"retained pending work"}],
            "capabilities":[{"capability_id":"cap","kind":"agent","target":"act","input_desc":"input", "output_desc":"output","definition":{},"version":"1"}]}},
        "result":null,"error":null,"events":[]
    })).unwrap();
    let mut cached = None;
    {
        let mut journal = worker.inner.journal.lock().await;
        journal.save(record).unwrap();
        journal
            .dispatch_payload("brain-root", &operation, &mut cached)
            .await
            .unwrap();
    }
    assert!(cached.is_some());
    worker.inner.report_cursor.lock().await.dispatch = cached;
    assert!(empty(&worker).await.unwrap());
    assert!(worker.inner.report_cursor.lock().await.dispatch.is_none());
    worker.shutdown().await.unwrap();
}

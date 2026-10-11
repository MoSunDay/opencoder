use super::*;

async fn run(fixture: &Fixture, run: DagClaimedRun) -> DagRunStatus {
    let (_sender, cancel) = tokio::sync::watch::channel(false);
    execute_run(
        RunDeps {
            uplink: fixture.uplink.clone(),
            exec: ExecDeps {
                store: fixture.store.clone(),
                workdir: fixture.workdir.clone(),
                config: fixture.config.clone(),
            },
            workflow_root: fixture.workflow_root.clone(),
        },
        run,
        cancel,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn retained_agent_events_are_reset_before_live_import_starts() {
    let (base, shared) = spawn_stub().await;
    let temporary = tempfile::tempdir().unwrap();
    let client = Arc::new(MockChatClient::new().push_script(vec![LlmEvent::Completed {
        text: "{\"current\":true}".into(),
        tool_calls: vec![],
        usage: None,
    }]));
    let fixture = fixture(&base, &temporary, client.clone()).await;
    let claimed = claimed(one_step_spec());
    let root = fixture.workflow_root.join(&claimed.run_id);
    let upper = root.join("upper/analyze");
    std::fs::create_dir_all(&upper).unwrap();
    // A previous attempt can leave a valid stream followed by a partial tail.
    // None of it belongs to this new child session or its reader offset.
    std::fs::write(
        upper.join("events.ndjson"),
        "{\"kind\":\"text_delta\",\"payload\":{\"text\":\"previous-attempt-only\"}}\ninvalid\n",
    )
    .unwrap();
    assert_eq!(run(&fixture, claimed).await, DagRunStatus::Done);
    await_status(&shared).await;
    assert_eq!(client.call_count(), 1);
    assert!(!shared
        .lock()
        .unwrap()
        .events
        .iter()
        .any(|event| { event.payload.to_string().contains("previous-attempt-only") }));
    let output: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("analyze/output.json")).unwrap()).unwrap();
    assert_eq!(output, json!({"current":true}));
    assert!(std::fs::read_dir(root.join("runc-state"))
        .unwrap()
        .next()
        .is_none());
}

#[tokio::test]
async fn invalid_live_agent_event_is_error_not_user_cancellation() {
    let (base, shared) = spawn_stub().await;
    let temporary = tempfile::tempdir().unwrap();
    let client = Arc::new(MockChatClient::new().push_script(vec![LlmEvent::Completed {
        text: String::new(),
        tool_calls: vec![opencoder_llm::CompletedToolCall {
            id: "corrupt-live-stream".into(),
            name: "bash".into(),
            input: json!({"command":"printf 'invalid\n' >> events.ndjson; sleep 5"}),
        }],
        usage: None,
    }]));
    let fixture = fixture(&base, &temporary, client).await;
    let claimed = claimed(one_step_spec());
    let root = fixture.workflow_root.join(&claimed.run_id);
    assert_eq!(run(&fixture, claimed).await, DagRunStatus::Error);
    await_status(&shared).await;
    let meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("analyze/meta.json")).unwrap()).unwrap();
    assert_eq!(meta["outcome"], "error");
    assert!(meta["error"]
        .as_str()
        .unwrap()
        .contains("invalid container event"));
    assert!(std::fs::read_dir(root.join("runc-state"))
        .unwrap()
        .next()
        .is_none());
}

#[tokio::test]
async fn event_stream_reset_rejects_symlink_without_mutating_target() {
    let (base, _) = spawn_stub().await;
    let temporary = tempfile::tempdir().unwrap();
    let client = Arc::new(MockChatClient::new());
    let fixture = fixture(&base, &temporary, client.clone()).await;
    let claimed = claimed(one_step_spec());
    let root = fixture.workflow_root.join(&claimed.run_id);
    let upper = root.join("upper/analyze");
    std::fs::create_dir_all(&upper).unwrap();
    let target = temporary.path().join("outside-stream");
    std::fs::write(&target, "must remain unchanged").unwrap();
    std::os::unix::fs::symlink(&target, upper.join("events.ndjson")).unwrap();
    assert_eq!(run(&fixture, claimed).await, DagRunStatus::Error);
    assert_eq!(client.call_count(), 0);
    assert_eq!(
        std::fs::read_to_string(target).unwrap(),
        "must remain unchanged"
    );
    let meta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("analyze/meta.json")).unwrap()).unwrap();
    assert!(meta["error"]
        .as_str()
        .unwrap()
        .contains("reset Agent event stream"));
}

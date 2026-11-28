use super::*;

/// The live driver applies the request on the writer it holds, acknowledges
/// it in the file, and the command reports the suspension it committed: one
/// interruption event carrying the request id, no writer on this side.
#[tokio::test]
async fn live_driver_applies_the_request_and_the_command_reports_it() {
    let workdir = tempfile::tempdir().unwrap();
    let workflow_id = "todos-live";
    let store = open_writer(workdir.path()).await;
    let hang = Arc::new(tokio::sync::Notify::new());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch_decision())
            .push_hang(hang.clone()),
    );
    let driver = runtime(&store, workdir.path(), mock);
    let spawned = tokio::spawn({
        let driver = driver.clone();
        async move {
            driver
                .run_new_with_id(spec(workflow_id), workflow_id.into())
                .await
        }
    });
    wait_until(&store, workflow_id, |s| {
        s.status == WorkflowStatus::Running
            && s.todos["step-1"].status == opencoder_todos::types::TodoStatus::Running
    })
    .await;

    let reported = tokio::time::timeout(
        LIVE_WAIT,
        dispatch_with(workdir.path(), workflow_id, LIVE_WAIT),
    )
    .await
    .expect("a live driver acknowledges within the bound")
    .unwrap();

    assert_eq!(reported.status, WorkflowStatus::Suspended);
    assert_eq!(reported.terminal_reason.as_deref(), Some(REASON));
    let acknowledged = pending_intent(workdir.path(), workflow_id).await.unwrap();
    assert_eq!(acknowledged.status, IntentStatus::Applied);
    assert_eq!(
        acknowledged.decided.unwrap().generation,
        reported.generation
    );
    let events = interrupt_events(&store, workflow_id).await;
    assert_eq!(events.len(), 1, "exactly one interruption: {events:?}");
    assert_eq!(events[0]["request_id"], acknowledged.request_id.as_str());
    assert_eq!(events[0]["reason"], REASON);
    assert_eq!(
        suspended_line(workflow_id, reported.generation),
        format!(
            "{workflow_id} suspended at generation {}",
            reported.generation
        )
    );

    hang.notify_one();
    let finished = tokio::time::timeout(LIVE_WAIT, spawned)
        .await
        .expect("the driver ends once the request is applied")
        .unwrap()
        .unwrap();
    assert_eq!(finished.status, WorkflowStatus::Suspended);
    assert_eq!(finished.generation, reported.generation);
}

/// The driver moved the generation before it looked at the request: it
/// marks the request stale instead of suspending, and the command retries
/// with a fresh request against the new generation. Only the retry is
/// committed; the stale request never reaches the event log.
#[tokio::test]
async fn stale_request_is_retried_against_the_fresh_generation() {
    let workdir = tempfile::tempdir().unwrap();
    let workflow_id = "todos-stale";
    seed(workdir.path(), workflow_id).await;
    let store = open_writer(workdir.path()).await;
    move_to_running(&store, workflow_id).await;
    let root = interrupt_intent::root(workdir.path());

    let command = tokio::spawn({
        let workdir = workdir.path().to_path_buf();
        async move { dispatch_with(&workdir, workflow_id, LIVE_WAIT).await }
    });
    let deadline = Instant::now() + LIVE_WAIT;
    let first = loop {
        if let Some(intent) = pending_intent(workdir.path(), workflow_id).await {
            break intent;
        }
        assert!(
            Instant::now() < deadline,
            "the command never published its request"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(first.status, IntentStatus::Requested);

    // The driver advances the workflow before its first look at the channel.
    let (spec, current) = persistence::load(&store, workflow_id)
        .await
        .unwrap()
        .unwrap();
    let mut advanced = current.clone();
    advanced.generation += 1;
    persistence::commit(&store, &spec, &advanced, "marker", serde_json::json!({}))
        .await
        .unwrap();

    let consumer = tokio::spawn({
        let store = store.clone();
        async move {
            loop {
                tokio::time::sleep(Duration::from_millis(20)).await;
                interrupt_intent::consume_pending(&store, &root, workflow_id)
                    .await
                    .unwrap();
            }
        }
    });
    let reported = tokio::time::timeout(LIVE_WAIT, command)
        .await
        .expect("the retried request is acknowledged")
        .expect("the command task must not panic")
        .unwrap();
    consumer.abort();

    assert_eq!(reported.status, WorkflowStatus::Suspended);
    assert_eq!(reported.terminal_reason.as_deref(), Some(REASON));
    assert_eq!(
        reported.generation,
        current.generation + 2,
        "stale, then the retry suspends"
    );
    let acknowledged = pending_intent(workdir.path(), workflow_id).await.unwrap();
    assert_ne!(
        acknowledged.request_id, first.request_id,
        "the retry is a new request"
    );
    assert_eq!(acknowledged.generation, current.generation + 1);
    assert_eq!(acknowledged.status, IntentStatus::Applied);
    let events = interrupt_events(&store, workflow_id).await;
    assert_eq!(
        events.len(),
        1,
        "only the retried request is committed: {events:?}"
    );
    assert_eq!(events[0]["request_id"], acknowledged.request_id.as_str());
}

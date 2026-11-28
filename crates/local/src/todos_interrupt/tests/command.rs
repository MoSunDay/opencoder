use super::*;

#[test]
fn suspended_line_names_the_workflow_and_generation() {
    assert_eq!(
        suspended_line("wf-line", 7),
        "wf-line suspended at generation 7"
    );
}

/// An unknown id is reported by name before anything is written: no intent,
/// no transition, no database (a reader never creates one).
#[tokio::test]
async fn unknown_workflow_is_reported_without_any_write() {
    let workdir = tempfile::tempdir().unwrap();
    let error = dispatch(workdir.path(), "todos-missing").await.unwrap_err();

    assert!(
        format!("{error:#}").contains("todo workflow not found: todos-missing"),
        "{error:#}"
    );
    assert!(
        !interrupt_intent::root(workdir.path()).exists(),
        "an unknown id publishes no intent"
    );
    assert!(
        !crate::session_cmd::database_for(workdir.path()).exists(),
        "an unknown id never creates the database"
    );
}

/// A terminal workflow is refused before anything is written: no intent is
/// published and no transition is committed.
#[tokio::test]
async fn terminal_workflow_is_refused_before_anything_is_published() {
    let workdir = tempfile::tempdir().unwrap();
    let workflow_id = "todos-terminal";
    seed(workdir.path(), workflow_id).await;
    let store = open_writer(workdir.path()).await;
    let (spec, initial) = persistence::load(&store, workflow_id)
        .await
        .unwrap()
        .unwrap();
    let failed =
        opencoder_todos::transitions::terminal(initial, WorkflowStatus::Failed, "done".into())
            .unwrap();
    persistence::commit(
        &store,
        &spec,
        &failed,
        "workflow_failed",
        serde_json::json!({}),
    )
    .await
    .unwrap();
    drop(store);

    let error = dispatch(workdir.path(), workflow_id).await.unwrap_err();
    let chain = format!("{error:#}");
    assert!(
        chain.contains("cannot interrupt terminal workflow"),
        "a terminal workflow is refused: {chain}"
    );
    let reader = reader(workdir.path()).await;
    assert_eq!(
        state(&reader, workflow_id).await.status,
        WorkflowStatus::Failed
    );
    assert!(interrupt_events(&reader, workflow_id).await.is_empty());
    assert!(pending_intent(workdir.path(), workflow_id).await.is_none());
}

/// A parked workflow needs no driver: the command reports the suspended state
/// without publishing a request or touching the generation.
#[tokio::test]
async fn parked_workflow_is_reported_without_a_request() {
    let workdir = tempfile::tempdir().unwrap();
    let workflow_id = "todos-parked";
    seed(workdir.path(), workflow_id).await;
    let store = open_writer(workdir.path()).await;
    park(&store, workflow_id, "earlier").await;
    let before = state(&store, workflow_id).await;
    drop(store);

    let reported = dispatch_with(workdir.path(), workflow_id, LIVE_WAIT)
        .await
        .unwrap();

    assert_eq!(
        serde_json::to_value(reported).unwrap(),
        serde_json::to_value(before).unwrap()
    );
    assert!(pending_intent(workdir.path(), workflow_id).await.is_none());
    assert_eq!(
        interrupt_events(&reader(workdir.path()).await, workflow_id)
            .await
            .len(),
        1
    );
}

/// A Running workflow with nothing driving it: the command waits its bound,
/// commits nothing itself, and leaves the durable request for the next
/// resume — which parks the workflow before scheduling anything. The
/// database's exclusive maintenance lease is held throughout; a writer
/// open would wait on it, and the command completed, so it opened none.
#[tokio::test]
async fn no_driver_commits_nothing_and_leaves_the_request_for_resume() {
    use opencoder_database::maintenance_lock;

    let workdir = tempfile::tempdir().unwrap();
    let workflow_id = "todos-undriven";
    seed(workdir.path(), workflow_id).await;
    let store = open_writer(workdir.path()).await;
    move_to_running(&store, workflow_id).await;
    let running = state(&store, workflow_id).await;
    drop(store);

    let database = crate::session_cmd::database_for(workdir.path());
    let key = database.canonicalize().unwrap();
    let lease = maintenance_lock::try_acquire_exclusive(&key).expect("no writer is alive");
    let error = tokio::time::timeout(
        LIVE_WAIT,
        dispatch_with(workdir.path(), workflow_id, Duration::from_millis(300)),
    )
    .await
    .expect("the command must not wait on a writer lease it never takes")
    .unwrap_err();
    let chain = format!("{error:#}");
    assert!(
        chain.contains("was not interrupted") && chain.contains("todos resume todos-undriven"),
        "the timeout is actionable: {chain}"
    );
    assert!(
        !chain.contains("suspended at generation"),
        "a queued request is never reported as a suspension: {chain}"
    );
    drop(lease);

    let reader = reader(workdir.path()).await;
    assert_eq!(
        serde_json::to_value(state(&reader, workflow_id).await).unwrap(),
        serde_json::to_value(running.clone()).unwrap(),
        "the command committed nothing"
    );
    let queued = pending_intent(workdir.path(), workflow_id)
        .await
        .expect("the request stays queued");
    assert_eq!(queued.status, IntentStatus::Requested);
    assert_eq!(queued.generation, running.generation);
    assert_eq!(queued.reason, REASON);

    // Takeover: the resume applies the queued request on its own writer.
    let store = open_writer(workdir.path()).await;
    let parked = runtime(&store, workdir.path(), Arc::new(MockChatClient::new()))
        .resume(workflow_id)
        .await
        .unwrap();
    assert_eq!(parked.status, WorkflowStatus::Suspended);
    assert_eq!(parked.terminal_reason.as_deref(), Some(REASON));
    let events = interrupt_events(&store, workflow_id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["request_id"], queued.request_id.as_str());
    assert_eq!(
        pending_intent(workdir.path(), workflow_id)
            .await
            .unwrap()
            .status,
        IntentStatus::Applied
    );
}

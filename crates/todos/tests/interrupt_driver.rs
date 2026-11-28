//! Live run/resume drivers consume lock-outside interrupt requests.

mod support;

use std::{sync::Arc, time::Duration};

use opencoder_llm::MockChatClient;
use opencoder_store::{Store, TursoStore};
use opencoder_todos::{
    interrupt_intent::{self, IntentStatus},
    persistence, transitions,
    types::*,
};

use support::{assert_interrupted_with, dispatch, one_todo_spec, runtime, seeded, state};

const REASON: &str = "intent channel test";

/// The live driver: a request published while a batch is in flight is
/// applied by the driver's watcher on the writer it owns. The run ends
/// Suspended with the request's reason and the event carries its id.
#[tokio::test]
async fn live_driver_applies_a_request_queued_mid_run() {
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    let hang = Arc::new(tokio::sync::Notify::new());
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(dispatch("step-1", "new"))
            .push_hang(hang.clone()),
    );
    let workdir = tempfile::tempdir().unwrap();
    let run_runtime = Arc::new(runtime(&store, mock, workdir.path()));
    let spawned = {
        let rt = run_runtime.clone();
        tokio::spawn(async move {
            rt.run_new_with_id(
                one_todo_spec("wf-live", "verify the live driver applies a queued request"),
                "wf-live".into(),
            )
            .await
        })
    };
    support::wait_until_running(&store, "wf-live").await;
    assert_eq!(
        state(&store, "wf-live").await.status,
        WorkflowStatus::Running
    );

    let root = interrupt_intent::root(workdir.path());
    let intent = interrupt_intent::write_request(
        &root,
        "wf-live",
        REASON,
        state(&store, "wf-live").await.generation,
    )
    .await
    .unwrap();

    let finished = tokio::time::timeout(Duration::from_secs(10), spawned)
        .await
        .expect("the run must end once the request is applied")
        .unwrap()
        .unwrap();
    hang.notify_one();
    assert_eq!(finished.status, WorkflowStatus::Suspended);
    assert_eq!(finished.terminal_reason.as_deref(), Some(REASON));
    assert_eq!(
        state(&store, "wf-live").await.status,
        WorkflowStatus::Suspended
    );
    assert_interrupted_with(&store, "wf-live", &intent.request_id).await;
    assert_eq!(
        interrupt_intent::read(&root, "wf-live")
            .await
            .unwrap()
            .unwrap()
            .status,
        IntentStatus::Applied
    );
}

/// A request that was queued while no driver was alive (the lock was held by
/// something else) is applied before a resume's first parent decision, so
/// the resume never starts work the user already asked to stop.
#[tokio::test]
async fn resume_applies_a_queued_request_before_the_first_decision() {
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    let (spec, initial) = seeded(&store, "wf-resume").await;
    let suspended =
        transitions::terminal(initial, WorkflowStatus::Suspended, "earlier".into()).unwrap();
    persistence::commit(
        &store,
        &spec,
        &suspended,
        "workflow_interrupted",
        serde_json::json!({"reason": "earlier"}),
    )
    .await
    .unwrap();

    let workdir = tempfile::tempdir().unwrap();
    let root = interrupt_intent::root(workdir.path());
    let intent = interrupt_intent::write_request(&root, "wf-resume", REASON, suspended.generation)
        .await
        .unwrap();
    // A resume that reached the model would dispatch; it must never get there.
    let mock = Arc::new(MockChatClient::new().push_script(dispatch("step-1", "resume")));

    let finished = runtime(&store, mock.clone(), workdir.path())
        .resume("wf-resume")
        .await
        .unwrap();
    assert_eq!(finished.status, WorkflowStatus::Suspended);
    assert_eq!(finished.terminal_reason.as_deref(), Some(REASON));
    assert_eq!(mock.call_count(), 0, "no parent decision may run");
    assert_interrupted_with(&store, "wf-resume", &intent.request_id).await;
    assert_eq!(
        interrupt_intent::read(&root, "wf-resume")
            .await
            .unwrap()
            .unwrap()
            .status,
        IntentStatus::Applied
    );
}

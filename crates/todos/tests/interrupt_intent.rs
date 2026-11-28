//! Lock-outside interrupt intents (`interrupt_intent`): the file channel's
//! own semantics, the driver-side consumption outcomes (apply, already
//! applied after a crash, terminal, unknown workflow) and the live driver
//! applying a queued request mid-run and before a resume's first decision.
//!
//! A second process cannot open the store while a driver owns the writer
//! lock, so the handshake is a file: `requested` by the interrupt writer,
//! `applied` by the driver after its commit. The database stays the only
//! source of truth; the `request_id` recorded in the interruption event is
//! what makes a crash between commit and mark idempotent.

mod support;

use std::{path::Path, sync::Arc};

use opencoder_store::{Store, TursoStore};
use opencoder_todos::{
    interrupt_intent::{self, ConsumeOutcome, Decision, IntentStatus, InterruptIntent},
    persistence, transitions,
    types::*,
};

use support::{assert_interrupted_with, kinds, seeded, state};

const REASON: &str = "intent channel test";

/// The file name is safe on every filesystem and two different ids never
/// share one: literal `%` escapes, so an id containing the escape of another
/// still encodes differently; over-long ids fall back to a stable digest.
#[test]
fn encoding_is_filename_safe_and_collision_free() {
    assert_eq!(
        interrupt_intent::encode_workflow_id("todos-01AZ.az_9"),
        "todos-01AZ.az_9"
    );
    let plain = interrupt_intent::encode_workflow_id("a/b");
    assert_eq!(plain, "a%2Fb");
    assert_ne!(plain, interrupt_intent::encode_workflow_id("a%2Fb"));
    assert_eq!(interrupt_intent::encode_workflow_id("a%2Fb"), "a%252Fb");
    assert!(!plain.contains('/'));

    let long = "L".repeat(300);
    let digest = interrupt_intent::encode_workflow_id(&long);
    assert!(digest.starts_with("long-"), "{digest}");
    assert_eq!(digest, interrupt_intent::encode_workflow_id(&long));
    assert_ne!(
        digest,
        interrupt_intent::encode_workflow_id(&"M".repeat(300))
    );

    let path = interrupt_intent::path(Path::new("/data/todos/.interrupts"), "a/b");
    assert_eq!(path, Path::new("/data/todos/.interrupts/a%2Fb.json"));
}

/// A request round-trips with a fresh id every time; `applied` is only ever
/// written back for the request the file still carries; removal is idempotent.
#[tokio::test]
async fn request_round_trip_marks_applied_only_for_its_own_id() {
    let root = tempfile::tempdir().unwrap();
    let first = interrupt_intent::write_request(root.path(), "wf-a", REASON, 0)
        .await
        .unwrap();
    let second = interrupt_intent::write_request(root.path(), "wf-a", REASON, 0)
        .await
        .unwrap();
    assert_ne!(first.request_id, second.request_id, "ids are never reused");
    assert_eq!(first.status, IntentStatus::Requested);
    assert!(first.requested_at_ms > 0);

    let read = interrupt_intent::read(root.path(), "wf-a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read, second, "the latest request replaces the earlier one");
    assert!(interrupt_intent::read(root.path(), "wf-b")
        .await
        .unwrap()
        .is_none());

    assert!(
        !interrupt_intent::mark(
            root.path(),
            "wf-a",
            &first.request_id,
            IntentStatus::Applied,
            Decision {
                generation: 0,
                at_ms: opencoder_core::message::now_ms()
            },
        )
        .await
        .unwrap(),
        "an acknowledgement for a stale id must not clobber the newer request"
    );
    assert_eq!(
        interrupt_intent::read(root.path(), "wf-a")
            .await
            .unwrap()
            .unwrap()
            .status,
        IntentStatus::Requested
    );
    assert!(interrupt_intent::mark(
        root.path(),
        "wf-a",
        &second.request_id,
        IntentStatus::Applied,
        Decision {
            generation: 0,
            at_ms: opencoder_core::message::now_ms()
        },
    )
    .await
    .unwrap());
    let applied = interrupt_intent::read(root.path(), "wf-a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(applied.status, IntentStatus::Applied);
    assert_eq!(applied.request_id, second.request_id);

    interrupt_intent::remove(root.path(), "wf-a").await.unwrap();
    interrupt_intent::remove(root.path(), "wf-a").await.unwrap();
    assert!(interrupt_intent::read(root.path(), "wf-a")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn consume_without_a_file_is_none() {
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    let root = tempfile::tempdir().unwrap();
    let outcome = interrupt_intent::consume_pending(&store, root.path(), "wf")
        .await
        .unwrap();
    assert!(matches!(outcome, ConsumeOutcome::None));
}

/// The driver-side half: a `requested` file is committed through the normal
/// transition path, the event carries the request id, the file is marked
/// `applied`, and a stale `applied` file is cleared on the next poll.
#[tokio::test]
async fn consume_commits_the_suspension_and_marks_the_file_applied() {
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    seeded(&store, "wf-consume").await;
    let root = tempfile::tempdir().unwrap();
    let intent = interrupt_intent::write_request(root.path(), "wf-consume", REASON, 0)
        .await
        .unwrap();

    let outcome = interrupt_intent::consume_pending(&store, root.path(), "wf-consume")
        .await
        .unwrap();
    let ConsumeOutcome::Applied(suspended) = outcome else {
        panic!("expected Applied, got {outcome:?}");
    };
    assert_eq!(suspended.status, WorkflowStatus::Suspended);
    assert_eq!(suspended.terminal_reason.as_deref(), Some(REASON));
    assert_eq!(
        state(&store, "wf-consume").await.status,
        WorkflowStatus::Suspended
    );
    assert_eq!(
        assert_interrupted_with(&store, "wf-consume", &intent.request_id).await,
        1
    );
    assert_eq!(
        interrupt_intent::read(root.path(), "wf-consume")
            .await
            .unwrap()
            .unwrap()
            .status,
        IntentStatus::Applied
    );

    // The next poll keeps the fresh acknowledgement for the waiting publisher.
    assert!(matches!(
        interrupt_intent::consume_pending(&store, root.path(), "wf-consume")
            .await
            .unwrap(),
        ConsumeOutcome::None
    ));
    let acknowledged = interrupt_intent::read(root.path(), "wf-consume")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(acknowledged.status, IntentStatus::Applied);
    assert!(acknowledged.decided.is_some());
    assert_eq!(
        kinds(&store, "wf-consume")
            .await
            .iter()
            .filter(|kind| *kind == "workflow_interrupted")
            .count(),
        1,
        "a consumed request is committed exactly once"
    );
}

/// Crash between commit and the applied rewrite: the file is still
/// `requested` although the event log already carries its id. The driver
/// must recognize that and mark it applied without a second commit.
#[tokio::test]
async fn consume_recognizes_a_crash_between_commit_and_mark() {
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    seeded(&store, "wf-crash").await;
    let root = tempfile::tempdir().unwrap();
    let intent = interrupt_intent::write_request(root.path(), "wf-crash", REASON, 0)
        .await
        .unwrap();
    // First driver commits, then "crashes" before the rewrite: the file is
    // restored to its requested form with the same id.
    assert!(matches!(
        interrupt_intent::consume_pending(&store, root.path(), "wf-crash")
            .await
            .unwrap(),
        ConsumeOutcome::Applied(_)
    ));
    interrupt_intent::write(
        root.path(),
        &InterruptIntent {
            status: IntentStatus::Requested,
            ..intent.clone()
        },
    )
    .await
    .unwrap();

    let outcome = interrupt_intent::consume_pending(&store, root.path(), "wf-crash")
        .await
        .unwrap();
    let ConsumeOutcome::AlreadyApplied(suspended) = outcome else {
        panic!("expected AlreadyApplied, got {outcome:?}");
    };
    assert_eq!(suspended.status, WorkflowStatus::Suspended);
    assert_eq!(
        interrupt_intent::read(root.path(), "wf-crash")
            .await
            .unwrap()
            .unwrap()
            .status,
        IntentStatus::Applied
    );
    assert_eq!(
        kinds(&store, "wf-crash")
            .await
            .iter()
            .filter(|kind| *kind == "workflow_interrupted")
            .count(),
        1
    );
}

/// A request that can never apply (terminal workflow, or no workflow at
/// all) is dropped rather than re-read on every poll.
#[tokio::test]
async fn consume_drops_requests_that_can_never_apply() {
    let store: Arc<dyn Store> = Arc::new(TursoStore::open_memory().await.unwrap());
    let root = tempfile::tempdir().unwrap();

    let (spec, initial) = seeded(&store, "wf-done").await;
    let failed = transitions::terminal(initial, WorkflowStatus::Failed, "done".into()).unwrap();
    persistence::commit(
        &store,
        &spec,
        &failed,
        "workflow_failed",
        serde_json::json!({}),
    )
    .await
    .unwrap();
    interrupt_intent::write_request(root.path(), "wf-done", REASON, 0)
        .await
        .unwrap();
    let outcome = interrupt_intent::consume_pending(&store, root.path(), "wf-done")
        .await
        .unwrap();
    assert!(
        matches!(outcome, ConsumeOutcome::Terminal(ref s) if s.status == WorkflowStatus::Failed),
        "{outcome:?}"
    );
    assert_eq!(
        interrupt_intent::read(root.path(), "wf-done")
            .await
            .unwrap()
            .unwrap()
            .status,
        IntentStatus::Terminal
    );
    assert_eq!(
        state(&store, "wf-done").await.status,
        WorkflowStatus::Failed
    );

    interrupt_intent::write_request(root.path(), "wf-missing", REASON, 0)
        .await
        .unwrap();
    assert!(matches!(
        interrupt_intent::consume_pending(&store, root.path(), "wf-missing")
            .await
            .unwrap(),
        ConsumeOutcome::None
    ));
    assert!(interrupt_intent::read(root.path(), "wf-missing")
        .await
        .unwrap()
        .is_none());
}

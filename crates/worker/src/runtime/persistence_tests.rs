//! Observable behavior of the self-healing persistence latch: bounded
//! backoff schedule, dispatch stays blocked inside the retry window, and a
//! healthy cycle clears both the worker and the project-deps latch.
use super::persistence;
use crate::{Worker, WorkerOptions};
use std::time::{Duration, Instant};

fn options(root: &std::path::Path) -> WorkerOptions {
    let workdir = root.join("work");
    std::fs::create_dir_all(&workdir).unwrap();
    WorkerOptions {
        name: "persistence-latch-test".into(),
        workdir,
        data_dir: root.join("node"),
        workflow_root: None,
        max_runs: Some(1),
        dag: false,
    }
}

#[test]
fn backoff_progresses_and_caps() {
    for (attempts, expected) in [
        (1, Duration::from_secs(1)),
        (2, Duration::from_secs(2)),
        (3, Duration::from_secs(4)),
        (4, Duration::from_secs(8)),
        (5, Duration::from_secs(16)),
        (6, Duration::from_secs(32)),
    ] {
        assert_eq!(
            persistence::backoff(attempts),
            expected,
            "attempt {attempts}"
        );
    }
    for attempts in [7u32, 8, 100, u32::MAX] {
        assert_eq!(persistence::backoff(attempts), persistence::RETRY_MAX);
    }
}

#[tokio::test]
async fn latched_worker_recovers_after_backoff_window() {
    let root = tempfile::tempdir().unwrap();
    let _config = opencoder_core::config::scoped_config_home(root.path().join("config"));
    let worker = Worker::open(options(root.path()), None).await.unwrap();
    // Keep the background scheduler out: it would run its own cycle and
    // clear the latch under test. dispatch_locked only assumes admission is
    // held somewhere, not that this caller holds it.
    let admission = worker.inner.admission.lock().await;

    persistence::record(&worker, "db unavailable".into());
    assert!(persistence::active(&worker));
    assert!(persistence::blocked(&worker));
    assert_eq!(
        worker.inner.persistence_retry.lock().unwrap().attempts,
        1,
        "record schedules the first backoff probe"
    );

    // Inside the retry window the cycle bails out before touching the store,
    // so the latch survives (dispatch stays blocked, nothing is started).
    assert_eq!(
        crate::operations::queue::dispatch_locked(&worker)
            .await
            .unwrap(),
        0
    );
    assert!(
        persistence::active(&worker),
        "a blocked cycle must not clear the latch"
    );
    assert!(persistence::blocked(&worker));

    // Force the probe deadline into the past: the next cycle runs against
    // the healthy store and proves persistence works again.
    worker.inner.persistence_retry.lock().unwrap().next_probe_at =
        Some(Instant::now() - Duration::from_secs(1));
    assert!(
        !persistence::blocked(&worker),
        "an expired deadline is probeable"
    );
    assert_eq!(
        crate::operations::queue::dispatch_locked(&worker)
            .await
            .unwrap(),
        0
    );
    assert!(
        !persistence::active(&worker),
        "a healthy cycle clears the latch"
    );
    assert!(!persistence::blocked(&worker));
    assert!(worker.inner.persistence_error.lock().unwrap().is_none());
    let retry = worker.inner.persistence_retry.lock().unwrap();
    assert_eq!(retry.attempts, 0, "the backoff schedule resets on recovery");
    assert!(retry.next_probe_at.is_none());

    worker.inner.stopping.cancel();
    drop(admission);
}

#[tokio::test]
async fn successful_dispatch_clears_the_project_dependencies_latch() {
    let root = tempfile::tempdir().unwrap();
    let _config = opencoder_core::config::scoped_config_home(root.path().join("config"));
    let worker = Worker::open(options(root.path()), None).await.unwrap();
    let admission = worker.inner.admission.lock().await;

    let deps = worker.inner.state.project.require().unwrap();
    *deps.persistence_error.lock().unwrap() = Some("project store down".into());
    drop(deps);
    // A deps-only latch carries no probe deadline: probeable immediately.
    assert!(!persistence::blocked(&worker));
    assert!(persistence::active(&worker));

    crate::operations::queue::dispatch_locked(&worker)
        .await
        .unwrap();

    assert!(
        worker
            .inner
            .state
            .project
            .require()
            .unwrap()
            .persistence_error
            .lock()
            .unwrap()
            .is_none(),
        "a successful cycle clears the deps-side latch too"
    );
    assert!(!persistence::active(&worker));

    worker.inner.stopping.cancel();
    drop(admission);
}

#[tokio::test]
async fn accessors_merge_own_then_deps_latch_into_snapshots() {
    let root = tempfile::tempdir().unwrap();
    let _config = opencoder_core::config::scoped_config_home(root.path().join("config"));
    let worker = Worker::open(options(root.path()), None).await.unwrap();
    let admission = worker.inner.admission.lock().await;

    // Healthy start: no latch text on either accessor, snapshot stays ready.
    assert_eq!(persistence::node_error(&worker), None);
    assert_eq!(persistence::error(&worker), None);
    assert!(
        opencoder_node::fleet::NodeService::snapshot(&worker).ready,
        "a healthy worker snapshots ready"
    );

    // Node latch wins the merge and is the text admission reports.
    persistence::record(&worker, "own store down".into());
    let deps = worker.inner.state.project.require().unwrap();
    *deps.persistence_error.lock().unwrap() = Some("project store down".into());
    drop(deps);
    assert_eq!(
        persistence::node_error(&worker).as_deref(),
        Some("own store down")
    );
    assert_eq!(
        persistence::error(&worker).as_deref(),
        Some("own store down")
    );
    let snapshot = opencoder_node::fleet::NodeService::snapshot(&worker);
    assert!(!snapshot.ready, "a latched worker must not look ready");
    assert_eq!(
        snapshot.resource_error.as_deref(),
        Some("own store down"),
        "the snapshot surface reports the merged node latch text"
    );

    // Clearing the node latch falls through to the deps latch text.
    *worker.inner.persistence_error.lock().unwrap() = None;
    assert_eq!(persistence::node_error(&worker), None);
    assert_eq!(
        persistence::error(&worker).as_deref(),
        Some("project store down")
    );
    let snapshot = opencoder_node::fleet::NodeService::snapshot(&worker);
    assert!(!snapshot.ready);
    assert_eq!(
        snapshot.resource_error.as_deref(),
        Some("project store down"),
        "a deps-only latch still reaches the snapshot surface"
    );

    worker.inner.stopping.cancel();
    drop(admission);
}

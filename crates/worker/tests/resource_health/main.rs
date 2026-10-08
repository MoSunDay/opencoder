#![cfg(not(windows))]
#[path = "../support/mod.rs"]
mod support;

use opencoder_node::fleet::NodeService;
use opencoder_worker::{Worker, WorkerRuntime};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};

#[tokio::test]
async fn blocked_mount_probe_does_not_block_snapshot_or_shutdown_or_spawn_more_probes() {
    let _config = support::isolated_config();
    let directory = tempfile::tempdir().unwrap();
    let (release, wait) = mpsc::channel();
    let wait = Arc::new(Mutex::new(wait));
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let runtime = WorkerRuntime {
        mount_health: Arc::new(move |path| {
            assert!(
                path.is_none(),
                "background probe must retain config isolation"
            );
            if count.fetch_add(1, Ordering::SeqCst) == 0 {
                wait.lock().unwrap().recv().unwrap();
            }
            Ok(())
        }),
        ..WorkerRuntime::default()
    };
    let worker = Worker::open_with_runtime(support::drain_options(directory.path()), None, runtime)
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while calls.load(Ordering::SeqCst) == 0 {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(5100)).await;
    let before = std::time::Instant::now();
    let snapshot = worker.snapshot();
    assert!(before.elapsed() < Duration::from_millis(100));
    assert!(!snapshot.ready);
    assert!(snapshot.resource_error.unwrap().contains("probe timed out"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    tokio::time::timeout(Duration::from_secs(2), worker.shutdown())
        .await
        .unwrap()
        .unwrap();
    release.send(()).unwrap();
}

#[tokio::test]
async fn late_success_is_not_healthy_until_a_fresh_probe_completes() {
    let _config = support::isolated_config();
    let directory = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let runtime = WorkerRuntime {
        mount_health: Arc::new(move |path| {
            assert!(
                path.is_none(),
                "background probe must retain config isolation"
            );
            if count.fetch_add(1, Ordering::SeqCst) == 0 {
                std::thread::sleep(Duration::from_millis(5200));
            }
            Ok(())
        }),
        ..WorkerRuntime::default()
    };
    let worker = Worker::open_with_runtime(support::drain_options(directory.path()), None, runtime)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(5500)).await;
    assert!(
        !worker.snapshot().ready,
        "late success must remain timed out"
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !worker.snapshot().ready {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(calls.load(Ordering::SeqCst) >= 2);
    worker.shutdown().await.unwrap();
}

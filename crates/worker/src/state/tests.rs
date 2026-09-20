use super::*;
use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Poll},
    time::Duration,
};

struct ReadyWhenReleased {
    worker: Option<Worker>,
    release: tokio::sync::oneshot::Receiver<()>,
    dropped: Arc<AtomicBool>,
}

impl Future for ReadyWhenReleased {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        Pin::new(&mut self.release).poll(cx).map(|_| ())
    }
}

impl Drop for ReadyWhenReleased {
    fn drop(&mut self) {
        drop(self.worker.take());
        self.dropped.store(true, Ordering::SeqCst);
    }
}

fn options(root: &std::path::Path) -> WorkerOptions {
    let workdir = root.join("work");
    std::fs::create_dir_all(workdir.join(".opencoder")).unwrap();
    std::fs::write(workdir.join(".opencoder/ap.json"), r#"{"mode":"off"}"#).unwrap();
    WorkerOptions {
        name: "task-tracker-test".into(),
        workdir,
        data_dir: root.join("node"),
        workflow_root: None,
        max_runs: Some(1),
        dag: false,
    }
}

#[tokio::test]
async fn stopping_scheduler_releases_node_ownership_while_admission_is_held() {
    let root = tempfile::tempdir().unwrap();
    let worker = Worker::open(options(root.path()), None).await.unwrap();
    let admission = worker.inner.admission.lock().await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while Arc::strong_count(&worker.inner) == 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("scheduler must be waiting for admission with a Worker capture");
    worker.inner.stopping.cancel();
    worker
        .wait_for_cleanup(tokio::time::Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(Arc::strong_count(&worker.inner), 1);
    drop(admission);
    drop(worker);
    let reopened = Worker::open(options(root.path()), None).await.unwrap();
    reopened.shutdown().await.unwrap();
}

#[tokio::test]
async fn shutdown_waits_for_the_entire_task_future_to_be_destroyed() {
    let root = tempfile::tempdir().unwrap();
    let worker = Worker::open(options(root.path()), None).await.unwrap();
    let dropped = Arc::new(AtomicBool::new(false));
    let (release, wait_for_release) = tokio::sync::oneshot::channel();
    worker.inner.tasks.spawn(ReadyWhenReleased {
        worker: Some(worker.clone()),
        release: wait_for_release,
        dropped: dropped.clone(),
    });

    {
        let shutdown = worker.shutdown();
        tokio::pin!(shutdown);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut shutdown)
                .await
                .is_err()
        );
        release.send(()).unwrap();
        shutdown.await.unwrap();
    }
    assert!(dropped.load(Ordering::SeqCst));

    drop(worker);
    let reopened = Worker::open(options(root.path()), None).await.unwrap();
    reopened.shutdown().await.unwrap();
}

#[tokio::test]
async fn idle_scheduler_does_not_keep_runtime_awake() {
    let root = tempfile::tempdir().unwrap();
    let worker = Worker::open(options(root.path()), None).await.unwrap();
    assert_eq!(worker.inner.background_tasks.active_count(), 1);
    assert!(worker.can_hibernate().await);
    worker.shutdown().await.unwrap();
    assert_eq!(worker.inner.background_tasks.active_count(), 0);
}

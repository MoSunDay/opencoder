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
    destructor: Option<DestructorGate>,
}

struct DestructorGate {
    entered: tokio::sync::oneshot::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

impl Future for ReadyWhenReleased {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        Pin::new(&mut self.release).poll(cx).map(|_| ())
    }
}

impl Drop for ReadyWhenReleased {
    fn drop(&mut self) {
        if let Some(gate) = self.destructor.take() {
            let _ = gate.entered.send(());
            let _ = gate.release.recv();
        }
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
        destructor: None,
    });

    {
        let shutdown = worker.shutdown();
        tokio::pin!(shutdown);
        assert!(futures::poll!(&mut shutdown).is_pending());
        release.send(()).unwrap();
        shutdown.await.unwrap();
    }
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(Arc::strong_count(&worker.inner), 1);

    drop(worker);
    let reopened = Worker::open(options(root.path()), None).await.unwrap();
    reopened.shutdown().await.unwrap();
}

async fn shutdown_waits_for_blocked_destructor(background: bool) {
    let root = tempfile::tempdir().unwrap();
    let worker = Worker::open(options(root.path()), None).await.unwrap();
    worker.inner.stopping.cancel();
    worker
        .inner
        .background_tasks
        .wait(tokio::time::Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    let dropped = Arc::new(AtomicBool::new(false));
    let (release, wait_for_release) = tokio::sync::oneshot::channel();
    let (entered, wait_for_destructor) = tokio::sync::oneshot::channel();
    let (finish_destructor, wait_for_finish) = std::sync::mpsc::channel();
    let task = ReadyWhenReleased {
        worker: Some(worker.clone()),
        release: wait_for_release,
        dropped: dropped.clone(),
        destructor: Some(DestructorGate {
            entered,
            release: wait_for_finish,
        }),
    };
    if background {
        worker.inner.background_tasks.spawn(task);
    } else {
        worker.inner.tasks.spawn(task);
    }
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), wait_for_destructor)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(Arc::strong_count(&worker.inner), 2);
    assert_eq!(worker.inner.tasks.active_count(), usize::from(!background));
    assert_eq!(
        worker.inner.background_tasks.active_count(),
        usize::from(background)
    );

    {
        let shutdown = worker.shutdown();
        tokio::pin!(shutdown);
        assert!(futures::poll!(&mut shutdown).is_pending());
        assert!(!dropped.load(Ordering::SeqCst));
        finish_destructor.send(()).unwrap();
        shutdown.await.unwrap();
    }
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(worker.inner.tasks.active_count(), 0);
    assert_eq!(worker.inner.background_tasks.active_count(), 0);
    assert_eq!(Arc::strong_count(&worker.inner), 1);
    drop(worker);
    let reopened = Worker::open(options(root.path()), None).await.unwrap();
    reopened.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_waits_for_execution_task_destructors() {
    shutdown_waits_for_blocked_destructor(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_waits_for_background_task_destructors() {
    shutdown_waits_for_blocked_destructor(true).await;
}

#[tokio::test]
async fn dropping_worker_releases_node_lock_despite_a_duplicated_descriptor() {
    let root = tempfile::tempdir().unwrap();
    let worker = Worker::open(options(root.path()), None).await.unwrap();
    let inherited_lock = worker.inner._lock.0.try_clone().unwrap();
    worker.shutdown().await.unwrap();
    assert_eq!(Arc::strong_count(&worker.inner), 1);
    assert!(Worker::open(options(root.path()), None).await.is_err());
    drop(worker);

    let reopened = Worker::open(options(root.path()), None).await.unwrap();
    assert!(fs2::FileExt::try_lock_exclusive(&inherited_lock).is_err());
    reopened.shutdown().await.unwrap();
    drop(reopened);
    drop(inherited_lock);
}

#[cfg(unix)]
struct ForkedProcess {
    process: libc::pid_t,
    release: std::os::unix::net::UnixStream,
}

#[cfg(unix)]
impl Drop for ForkedProcess {
    fn drop(&mut self) {
        self.release.shutdown(std::net::Shutdown::Write).unwrap();
        let mut status = 0;
        let waited = unsafe { libc::waitpid(self.process, &mut status, 0) };
        assert_eq!(waited, self.process);
        assert_eq!(status, 0);
    }
}

#[cfg(unix)]
fn fork_inheriting_descriptors() -> ForkedProcess {
    use std::os::fd::AsRawFd;

    let (release, wait) = std::os::unix::net::UnixStream::pair().unwrap();
    let release_fd = release.as_raw_fd();
    let wait_fd = wait.as_raw_fd();
    let process = unsafe { libc::fork() };
    if process == 0 {
        let mut buffer = [0_u8; 1];
        unsafe {
            libc::close(release_fd);
            libc::read(wait_fd, buffer.as_mut_ptr().cast(), buffer.len());
            libc::_exit(0);
        }
    }
    assert!(
        process > 0,
        "fork failed: {}",
        std::io::Error::last_os_error()
    );
    drop(wait);
    ForkedProcess { process, release }
}

#[cfg(unix)]
#[tokio::test]
async fn dropping_worker_releases_node_lock_while_a_forked_child_keeps_its_descriptor() {
    let root = tempfile::tempdir().unwrap();
    let worker = Worker::open(options(root.path()), None).await.unwrap();
    let child = fork_inheriting_descriptors();
    worker.shutdown().await.unwrap();
    assert_eq!(Arc::strong_count(&worker.inner), 1);
    drop(worker);

    let reopened = Worker::open(options(root.path()), None).await.unwrap();
    drop(child);
    assert!(Worker::open(options(root.path()), None).await.is_err());
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

#[tokio::test]
async fn terminal_release_collects_unused_gates_and_maintenance_registrations() {
    let root = tempfile::tempdir().unwrap();
    let worker = Worker::open(options(root.path()), None).await.unwrap();
    let id = "agent-release";
    let held = worker.lifecycle_gate(id).await;
    worker
        .inner
        .lifecycle_gates
        .lock()
        .await
        .insert("agent-orphan".into(), Arc::new(Mutex::new(())));
    worker
        .inner
        .preparation_gates
        .lock()
        .await
        .insert(id.into(), Arc::new(Mutex::new(())));
    crate::maintenance_tools::install(&worker, id);
    assert!(worker.inner.maintenance.lock().unwrap().contains_key(id));

    release_execution_state(&worker.inner, id).await;
    // The held clone keeps the in-use gate alive; unreferenced entries are
    // collected and the maintenance registration drops its global slot.
    assert!(worker.inner.lifecycle_gates.lock().await.contains_key(id));
    assert!(!worker
        .inner
        .lifecycle_gates
        .lock()
        .await
        .contains_key("agent-orphan"));
    assert!(!worker.inner.preparation_gates.lock().await.contains_key(id));
    assert!(!worker.inner.maintenance.lock().unwrap().contains_key(id));

    drop(held);
    release_execution_state(&worker.inner, id).await;
    assert!(!worker.inner.lifecycle_gates.lock().await.contains_key(id));
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn configuration_cache_reuses_the_load_until_input_files_change() {
    let root = tempfile::tempdir().unwrap();
    let _home = opencoder_core::config::scoped_config_home(root.path().join("home"));
    let worker = Worker::open(options(root.path()), None).await.unwrap();
    let first = configuration_shared(&worker).unwrap();
    let second = configuration_shared(&worker).unwrap();
    assert!(
        Arc::ptr_eq(&first, &second),
        "unchanged inputs must reuse the cached config"
    );

    // Calls inside a harness scope keep loading fresh (core folds those
    // call-site overrides into the loaded config).
    let scoped = opencoder_core::harness::scope::with_execution(
        None,
        opencoder_core::harness::RuntimeSettings::default(),
        async { configuration_shared(&worker).unwrap() },
    )
    .await;
    assert!(!Arc::ptr_eq(&first, &scoped));

    // Rewriting a covered project candidate invalidates the cache.
    let config = root.path().join("work/.opencoder/config.json");
    opencoder_core::atomic_write(&config, br#"{"model":"probe/cache-invalidation"}"#).unwrap();
    let third = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&first, &third));
    assert_eq!(third.model, "probe/cache-invalidation");
    assert_eq!(third.model, worker.configuration().unwrap().model);
    worker.shutdown().await.unwrap();
}

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
    std::fs::create_dir_all(&workdir).unwrap();
    WorkerOptions {
        name: "task-tracker-test".into(),
        workdir,
        data_dir: root.join("node"),
        workflow_root: None,
        max_runs: Some(1),
        dag: false,
    }
}

fn write_json(path: &std::path::Path, value: serde_json::Value) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, value.to_string()).unwrap();
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

#[tokio::test]
async fn configuration_cache_tracks_active_env_transitions_and_files() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let _home = opencoder_core::config::scoped_config_home(home.clone());
    let global = home.join(".opencoder");
    write_json(
        &global.join("config.json"),
        serde_json::json!({"model": "base/base-model"}),
    );
    write_json(
        &global.join("cli.json"),
        serde_json::json!({"base-cli": {"enabled": true, "content": "base"}}),
    );
    write_json(
        &global.join("skills.json"),
        serde_json::json!({"base-skill": {"enabled": true}}),
    );
    let worker = Worker::open(options(root.path()), None).await.unwrap();
    let inactive = configuration_shared(&worker).unwrap();
    assert_eq!(inactive.model, "base/base-model");

    let env_a = opencoder_core::create_config_env("a").unwrap();
    let env_b = opencoder_core::create_config_env("b").unwrap();
    for (env, model, cli, skill) in [
        (&env_a, "env/a-one", "a-cli", "a-skill"),
        (&env_b, "env/b-one", "b-cli", "b-skill"),
    ] {
        write_json(
            &env.path.join("config.json"),
            serde_json::json!({"model": model}),
        );
        write_json(
            &env.path.join("cli.json"),
            serde_json::json!({(cli): {"enabled": true, "content": "before"}}),
        );
        write_json(
            &env.path.join("skills.json"),
            serde_json::json!({skill: {"enabled": true}}),
        );
    }

    opencoder_core::activate_config_env("a").unwrap();
    let first_a = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&inactive, &first_a));
    assert_eq!(first_a.model, "env/a-one");
    assert!(Arc::ptr_eq(
        &first_a,
        &configuration_shared(&worker).unwrap()
    ));

    opencoder_core::activate_config_env("b").unwrap();
    let first_b = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&first_a, &first_b));
    assert_eq!(first_b.model, "env/b-one");
    assert_eq!(first_b.enabled_cli()[0].0, "b-cli");
    assert!(first_b.skills.contains_key("b-skill"));

    opencoder_core::activate_config_env("a").unwrap();
    let second_a = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&first_b, &second_a));
    assert_eq!(second_a.model, "env/a-one");
    assert_eq!(second_a.enabled_cli()[0].0, "a-cli");
    assert!(second_a.skills.contains_key("a-skill"));

    // Keep each rewrite the same byte length so invalidation exercises mtime,
    // not just file-size changes.
    std::thread::sleep(Duration::from_millis(5));
    write_json(
        &env_a.path.join("config.json"),
        serde_json::json!({"model": "env/a-two"}),
    );
    let edited_config = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&second_a, &edited_config));
    assert_eq!(edited_config.model, "env/a-two");

    std::thread::sleep(Duration::from_millis(5));
    write_json(
        &env_a.path.join("cli.json"),
        serde_json::json!({"a-cli": {"enabled": true, "content": "after!"}}),
    );
    let edited_cli = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&edited_config, &edited_cli));
    assert_eq!(edited_cli.enabled_cli()[0].1.content, "after!");

    std::thread::sleep(Duration::from_millis(5));
    write_json(
        &env_a.path.join("skills.json"),
        serde_json::json!({"b-skill": {"enabled": true}}),
    );
    let edited_skills = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&edited_cli, &edited_skills));
    assert!(!edited_skills.skills.contains_key("a-skill"));
    assert!(edited_skills.skills.contains_key("b-skill"));

    std::fs::remove_file(env_a.path.join("cli.json")).unwrap();
    let removed_cli = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&edited_skills, &removed_cli));
    assert_eq!(removed_cli.enabled_cli()[0].0, "base-cli");

    std::fs::remove_file(env_a.path.join("skills.json")).unwrap();
    let removed_skills = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&removed_cli, &removed_skills));
    assert!(removed_skills.skills.contains_key("base-skill"));

    std::fs::remove_file(env_a.path.join("config.json")).unwrap();
    let removed_config = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&removed_skills, &removed_config));
    assert_eq!(removed_config.model, "base/base-model");

    opencoder_core::deactivate_config_env().unwrap();
    let deactivated = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&removed_config, &deactivated));
    assert_eq!(deactivated.model, "base/base-model");

    opencoder_core::activate_config_env("a").unwrap();
    let reactivated = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&deactivated, &reactivated));
    let marker = opencoder_core::config_env_status().unwrap().marker_path;
    std::fs::write(&marker, "missing\n").unwrap();
    let stale = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&reactivated, &stale));
    assert!(opencoder_core::config_env_status().unwrap().stale_marker);
    assert_eq!(stale.model, "base/base-model");

    opencoder_core::deactivate_config_env().unwrap();
    let stale_removed = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&stale, &stale_removed));
    assert_eq!(stale_removed.model, "base/base-model");

    // Even an exact cache-key match must not hide a marker read error. A
    // directory gives a deterministic non-NotFound error on every platform.
    std::fs::create_dir(&marker).unwrap();
    assert!(configuration_shared(&worker).is_err());
    std::fs::remove_dir(&marker).unwrap();
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn configuration_cache_missing_to_directory_forces_a_real_load() {
    let root = tempfile::tempdir().unwrap();
    let _home = opencoder_core::config::scoped_config_home(root.path().join("home"));
    let worker = Worker::open(options(root.path()), None).await.unwrap();
    let cached = configuration_shared(&worker).unwrap();
    assert!(Arc::ptr_eq(
        &cached,
        &configuration_shared(&worker).unwrap()
    ));

    // A directory was previously omitted from the fingerprint just like a
    // missing file. It must invalidate the key and reach Config::load, which
    // reports the read error instead of returning the stale cached value.
    let config = root.path().join("work/opencoder.json");
    std::fs::create_dir(&config).unwrap();
    assert!(configuration_shared(&worker).is_err());

    std::fs::remove_dir(&config).unwrap();
    let recovered = configuration_shared(&worker).unwrap();
    assert!(Arc::ptr_eq(&cached, &recovered));
    worker.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn configuration_cache_propagates_metadata_errors_instead_of_reusing_stale_config() {
    let root = tempfile::tempdir().unwrap();
    let _home = opencoder_core::config::scoped_config_home(root.path().join("home"));
    let worker = Worker::open(options(root.path()), None).await.unwrap();
    let cached = configuration_shared(&worker).unwrap();

    // A self-referential symlink makes stat fail with ELOOP without relying
    // on permission behavior (tests may run as a privileged user).
    let config = root.path().join("work/opencoder.json");
    std::os::unix::fs::symlink("opencoder.json", &config).unwrap();
    assert!(configuration_shared(&worker).is_err());

    std::fs::remove_file(&config).unwrap();
    let recovered = configuration_shared(&worker).unwrap();
    assert!(Arc::ptr_eq(&cached, &recovered));
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn configuration_cache_tracks_xdg_cli_and_skill_creation_edits_and_removals() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let _home = opencoder_core::config::scoped_config_home(home.clone());
    let xdg = home.join("opencoder");
    write_json(
        &xdg.join("config.json"),
        serde_json::json!({"model": "xdg/base-model"}),
    );
    let worker = Worker::open(options(root.path()), None).await.unwrap();
    let base = configuration_shared(&worker).unwrap();
    assert!(base.cli.is_empty());
    assert!(base.skills.is_empty());

    let cli = xdg.join("cli.json");
    let skills = xdg.join("skills.json");
    write_json(
        &cli,
        serde_json::json!({"xdg-cli": {"enabled": true, "content": "one"}}),
    );
    let added_cli = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&base, &added_cli));
    assert_eq!(added_cli.enabled_cli()[0].0, "xdg-cli");

    write_json(&skills, serde_json::json!({"old-skill": {"enabled": true}}));
    let added_skills = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&added_cli, &added_skills));
    assert!(added_skills.skills.contains_key("old-skill"));

    // Same-length rewrites prove these cases exercise metadata invalidation,
    // not only a change in file size.
    std::thread::sleep(Duration::from_millis(5));
    write_json(
        &cli,
        serde_json::json!({"xdg-cli": {"enabled": true, "content": "two"}}),
    );
    let edited_cli = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&added_skills, &edited_cli));
    assert_eq!(edited_cli.enabled_cli()[0].1.content, "two");

    std::thread::sleep(Duration::from_millis(5));
    write_json(&skills, serde_json::json!({"new-skill": {"enabled": true}}));
    let edited_skills = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&edited_cli, &edited_skills));
    assert!(!edited_skills.skills.contains_key("old-skill"));
    assert!(edited_skills.skills.contains_key("new-skill"));

    std::fs::remove_file(&cli).unwrap();
    let removed_cli = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&edited_skills, &removed_cli));
    assert!(removed_cli.cli.is_empty());
    assert!(removed_cli.skills.contains_key("new-skill"));

    std::fs::remove_file(&skills).unwrap();
    let removed_skills = configuration_shared(&worker).unwrap();
    assert!(!Arc::ptr_eq(&removed_cli, &removed_skills));
    assert!(removed_skills.skills.is_empty());
    worker.shutdown().await.unwrap();
}

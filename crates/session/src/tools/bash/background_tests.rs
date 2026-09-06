use super::*;

#[test]
fn output_path_format() {
    let p = output_path(12345);
    assert_eq!(p.to_str().unwrap(), "/tmp/opencoder_bg_12345.output");
}

/// `register` adds a live entry that `list` exposes; `unregister` removes
/// it while leaving the process alive (verified by `stop`-free kill).
#[cfg(unix)]
#[tokio::test]
async fn register_unregister_roundtrip() {
    use std::process::Command;
    use std::time::Duration;

    let _g = test_registry_mutex().lock().await;
    let mut child = Command::new("setsid")
        .args(["sleep", "60"])
        .spawn()
        .expect("spawn setsid sleep");
    let pid = child.id();
    let pgid = pid as libc::pid_t;
    std::thread::sleep(Duration::from_millis(50));

    register(pid, pgid, "test".to_string());
    assert!(
        list().iter().any(|info| info.pid == pid),
        "list should expose the registered pid"
    );

    // unregister removes the entry without touching the process.
    unregister(pid);
    assert!(
        !list().iter().any(|info| info.pid == pid),
        "list should no longer contain the pid after unregister"
    );
    // idempotent: unregistering again is a no-op.
    unregister(pid);

    // Reap the still-alive child directly (never registered for /stop).
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }
    let _ = child.wait();
}

/// `stop(pid)` kills the registered process group, removes the entry, and
/// reports `false` once the entry is gone.
#[cfg(unix)]
#[test]
fn stop_kills_registered_process() {
    use std::process::Command;
    use std::time::Duration;

    let _g = test_registry_mutex().blocking_lock();
    let mut child = Command::new("setsid")
        .args(["sleep", "60"])
        .spawn()
        .expect("spawn setsid sleep");
    let pid = child.id();
    let pgid = pid as libc::pid_t;
    std::thread::sleep(Duration::from_millis(50));

    register(pid, pgid, "test".to_string());
    assert!(stop(pid), "stop should find the registered pid");
    assert!(
        !list().iter().any(|info| info.pid == pid),
        "stop should remove the registry entry"
    );
    assert!(!stop(pid), "second stop finds nothing");

    // The child was killed by stop(); reap the zombie.
    let _ = child.wait();
}

/// `kill_all()` drains the whole registry: it SIGKILLs every registered
/// process group, removes every entry, and returns the number killed.
#[cfg(unix)]
#[test]
fn kill_all_terminates_every_registered_process() {
    use std::process::Command;
    use std::time::Duration;

    let _g = test_registry_mutex().blocking_lock();
    // Drain entries any earlier test may have leaked so the count below is
    // deterministic. The mutex guarantees no other registry test is live,
    // and SIGKILLing already-orphaned `sleep` groups is harmless.
    let leaked = kill_all();
    assert!(
        list().is_empty(),
        "registry must be empty after drain (leaked {leaked})"
    );

    let mut children: Vec<std::process::Child> = Vec::new();
    for _ in 0..2 {
        let child = Command::new("setsid")
            .args(["sleep", "60"])
            .spawn()
            .expect("spawn setsid sleep");
        let pid = child.id();
        let pgid = pid as libc::pid_t;
        std::thread::sleep(Duration::from_millis(50));
        register(pid, pgid, "test".to_string());
        children.push(child);
    }
    assert_eq!(list().len(), 2, "both processes should be registered");

    let killed = kill_all();
    assert_eq!(killed, 2, "kill_all should report the number it killed");
    assert!(
        list().is_empty(),
        "kill_all should drain the entire registry"
    );

    // Reap the children that kill_all() SIGKILLed.
    for child in children.iter_mut() {
        let _ = child.wait();
    }
}

#[test]
fn bg_state_push_buffers_when_no_file() {
    let mut st = BgState::new();
    assert!(st.push_stdout(b"hello"));
    assert!(st.push_stderr(b"world"));
    assert_eq!(&st.stdout_buf, b"hello");
    assert_eq!(&st.stderr_buf, b"world");
    assert!(st.file.is_none());
}

#[test]
fn capture_accepts_exact_limit_and_rejects_next_byte() {
    let mut stdout = BgState::new();
    assert!(stdout.push_stdout(&vec![b'x'; STREAM_OUTPUT_LIMIT_BYTES]));
    assert_eq!(stdout.stdout_buf.len(), STREAM_OUTPUT_LIMIT_BYTES);
    assert!(!stdout.push_stdout(b"x"));
    assert_eq!(stdout.stdout_buf.len(), STREAM_OUTPUT_LIMIT_BYTES);
    assert_eq!(
        stdout.output_limit_error(),
        Some("output_limit_exceeded: bash stdout exceeds 8388608 bytes")
    );

    let mut stderr = BgState::new();
    assert!(stderr.push_stderr(&vec![b'x'; STREAM_OUTPUT_LIMIT_BYTES]));
    assert!(!stderr.push_stderr(b"x"));
    assert_eq!(stderr.stderr_buf.len(), STREAM_OUTPUT_LIMIT_BYTES);
    assert_eq!(
        stderr.output_limit_error(),
        Some("output_limit_exceeded: bash stderr exceeds 8388608 bytes")
    );
}

#[test]
fn background_file_stops_growing_after_overflow() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("background.output");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    let mut state = BgState::new();
    state.file = Some(file);
    assert!(state.push_stdout(&vec![b'x'; STREAM_OUTPUT_LIMIT_BYTES]));
    assert!(!state.push_stdout(b"x"));
    let overflow_size = std::fs::metadata(&path).unwrap().len();
    assert!(!state.push_stdout(&vec![b'x'; 1024]));
    assert_eq!(std::fs::metadata(path).unwrap().len(), overflow_size);
}

#[cfg(unix)]
#[tokio::test]
async fn completed_handoff_removes_handle_and_retains_output() {
    let _guard = test_registry_mutex().lock().await;
    cleanup_all();

    let child = tokio::process::Command::new("setsid")
        .args(["sh", "-c", "exit 0"])
        .spawn()
        .expect("spawn setsid sleep");
    let pid = child.id().expect("pid");
    let pgid = pid as libc::pid_t;
    let process_group = ProcessGroupGuard::registered(pid, pgid, "test-session".into());

    let state = std::sync::Arc::new(std::sync::Mutex::new(BgState::new()));
    let stdout_task = tokio::spawn(async {});
    let stderr_task = tokio::spawn(async {});

    handoff(pid, child, stdout_task, stderr_task, state, process_group)
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(5), async {
        while all_task_handles_len_for_test() != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("completed supervisor removes its handle");
    assert_eq!(all_task_handles_len_for_test(), 0);
    assert_eq!(retained_outputs_len_for_test(), 1);
    assert!(output_path(pid).exists(), "recent output remains readable");
    cleanup_all();
    assert!(!output_path(pid).exists());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn handoff_limit_rejects_and_cleans_unaccepted_process() {
    let _guard = test_registry_mutex().lock().await;
    cleanup_all();

    let spawn = |label: &str| {
        let child = tokio::process::Command::new("setsid")
            .args(["sleep", "300"])
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let guard = ProcessGroupGuard::registered(pid, pid as libc::pid_t, label.into());
        (pid, child, guard)
    };
    let (first_pid, first, first_guard) = spawn("first");
    handoff_with_limit(
        first_pid,
        first,
        tokio::spawn(async {}),
        tokio::spawn(async {}),
        std::sync::Arc::new(std::sync::Mutex::new(BgState::new())),
        first_guard,
        1,
    )
    .await
    .unwrap();
    let (rejected_pid, rejected, rejected_guard) = spawn("rejected");
    let error = handoff_with_limit(
        rejected_pid,
        rejected,
        tokio::spawn(async {}),
        tokio::spawn(async {}),
        std::sync::Arc::new(std::sync::Mutex::new(BgState::new())),
        rejected_guard,
        1,
    )
    .await
    .unwrap_err();
    assert!(
        error.contains("background_process_limit_exceeded"),
        "{error}"
    );
    assert!(
        !list().iter().any(|entry| entry.pid == rejected_pid),
        "rejected process must be unregistered by its guard"
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while std::path::Path::new("/proc")
            .join(rejected_pid.to_string())
            .exists()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("rejected handoff process must be terminated and reaped");
    cleanup_all();
}

#[test]
fn completed_output_retention_is_count_and_time_bounded() {
    let _guard = test_registry_mutex().blocking_lock();
    cleanup_all();
    begin_background_lifecycle();
    let dir = tempfile::tempdir().unwrap();
    let now = Instant::now();
    let mut paths = Vec::new();
    for index in 0..(MAX_RETAINED_OUTPUTS + 2) {
        let path = dir.path().join(format!("{index}.output"));
        std::fs::write(&path, b"output").unwrap();
        retain_completed_output(path.clone(), now);
        paths.push(path);
    }
    assert_eq!(retained_outputs_len_for_test(), MAX_RETAINED_OUTPUTS);
    assert!(!paths[0].exists() && !paths[1].exists());
    assert!(paths[2..].iter().all(|path| path.exists()));

    prune_completed_outputs(now + COMPLETED_OUTPUT_TTL + Duration::from_millis(1));
    assert_eq!(retained_outputs_len_for_test(), 0);
    assert!(paths.iter().all(|path| !path.exists()));
    cleanup_all();
}

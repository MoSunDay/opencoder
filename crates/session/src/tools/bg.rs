//! Global process registry for the bash tool.
//!
//! Every bash command registers itself on spawn so the display-only `/ps`
//! command can list the running process and `/stop` can kill its process
//! group. A bash command runs in the foreground until it exits naturally;
//! [`register`] adds the entry, [`unregister`] removes it on completion, and
//! [`stop`]/[`kill_all`] terminate the group on user demand. The legacy
//! [`handoff`] path (detached background supervisor that streamed output to a
//! temp file) is retained for reference but is no longer reached from the
//! foreground tool.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tokio::process::Child;
use tokio::task::JoinHandle;

/// Shared capture state for a backgrounded command's stdout/stderr.
///
/// In the foreground phase (`file == None`) it only buffers into `stdout_buf`
/// / `stderr_buf`. After [`handoff`] sets `file`, subsequent `push_*` calls
/// also append to the file so the background output file stays live.
pub struct BgState {
    pub stdout_buf: Vec<u8>,
    pub stderr_buf: Vec<u8>,
    file: Option<std::fs::File>,
}

impl BgState {
    pub fn new() -> Self {
        Self {
            stdout_buf: Vec::new(),
            stderr_buf: Vec::new(),
            file: None,
        }
    }

    /// Append a chunk of stdout. Writes to the file (if handed off) under the
    /// same lock — no await is held.
    pub fn push_stdout(&mut self, data: &[u8]) {
        self.stdout_buf.extend_from_slice(data);
        if let Some(f) = &mut self.file {
            let _ = f.write_all(data);
        }
    }

    /// Append a chunk of stderr. Same file-write semantics as `push_stdout`.
    pub fn push_stderr(&mut self, data: &[u8]) {
        self.stderr_buf.extend_from_slice(data);
        if let Some(f) = &mut self.file {
            let _ = f.write_all(data);
        }
    }
}

impl Default for BgState {
    fn default() -> Self {
        Self::new()
    }
}

/// Path of the background output file for a given pid.
pub fn output_path(pid: u32) -> PathBuf {
    PathBuf::from(format!("/tmp/opencoder_bg_{pid}.output"))
}

struct BgEntry {
    pgid: libc::pid_t,
    #[allow(dead_code)]
    session_id: String,
    output_path: PathBuf,
}

fn registry() -> &'static Mutex<HashMap<u32, BgEntry>> {
    static REG: OnceLock<Mutex<HashMap<u32, BgEntry>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Hand a timed-out command to a detached background supervisor.
///
/// Opens/truncates the output file, flushes the already-captured stdout/stderr
/// buffers to it, sets `state.file` so subsequent incremental pushes go
/// straight to the file, registers the entry, and spawns a detached task that:
///
/// 1. Waits for the child to exit naturally (kill_on_drop is defanged because
///    we own the `Child` and only drop it after `wait()`).
/// 2. Awaits both drain tasks until EOF so the file captures the full output.
/// 3. Appends `[exit code: N]`.
/// 4. `kill(-pgid, SIGKILL)` to clean up any lingering process-group members.
/// 5. Removes the registry entry.
#[allow(clippy::too_many_arguments)]
pub fn handoff(
    pid: u32,
    pgid: libc::pid_t,
    session_id: String,
    mut child: Child,
    stdout_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
    state: std::sync::Arc<Mutex<BgState>>,
) {
    let path = output_path(pid);

    // Open/truncate + flush captured buffers + activate file mode.
    {
        let mut file = match OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(pid, error = %e, "bg: open output file failed");
                // Best-effort: still kill the group so we don't leak processes.
                #[cfg(unix)]
                unsafe {
                    let _ = libc::kill(-pgid, libc::SIGKILL);
                }
                return;
            }
        };
        let mut st = state.lock().unwrap();
        let _ = file.write_all(&st.stdout_buf);
        if !st.stderr_buf.is_empty() {
            let _ = file.write_all(b"\n[stderr]\n");
            let _ = file.write_all(&st.stderr_buf);
        }
        st.file = Some(file);
    }

    // Register.
    {
        let mut reg = registry().lock().unwrap();
        reg.insert(
            pid,
            BgEntry {
                pgid,
                session_id,
                output_path: path.clone(),
            },
        );
    }

    // Detached supervisor — owns `child` so kill_on_drop won't fire early.
    tokio::spawn(async move {
        let exit_status = child.wait().await;

        // Kill the entire process group NOW (before awaiting drain tasks) so
        // that any grandchildren holding the pipe write-ends die and the drain
        // tasks can reach EOF. Without this, a backgrounded descendant would
        // keep the pipes open and the awaits below would block forever.
        #[cfg(unix)]
        unsafe {
            let _ = libc::kill(-pgid, libc::SIGKILL);
        }

        // Bounded wait for drain tasks: after the group kill the pipe
        // write-ends close and the tasks resolve with EOF. A 2s ceiling
        // guards against a process that escaped the group kill.
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            let _ = stdout_task.await;
            let _ = stderr_task.await;
        })
        .await;

        let code = exit_status.ok().and_then(|s| s.code()).unwrap_or(-1);

        // Only annotate non-zero exits; success (code == 0) is implicit and
        // would just add noise to the background output file.
        if code != 0 {
            if let Ok(mut f) = OpenOptions::new().append(true).open(&path) {
                let _ = write!(f, "\n[exit code: {code}]");
            }
        }

        let mut reg = registry().lock().unwrap();
        reg.remove(&pid);
    });
}

/// Public snapshot of one registered background process, for display-only
/// commands such as the TUI `/ps`. Carries only the public fields — never the
/// raw `Child`/handles owned by the detached supervisor.
#[derive(Clone, Debug)]
pub struct BgInfo {
    pub pid: u32,
    pub output_path: PathBuf,
}

/// Snapshot every registered background process into public [`BgInfo`]s.
pub fn list() -> Vec<BgInfo> {
    let reg = registry().lock().unwrap();
    reg.iter()
        .map(|(pid, e)| BgInfo {
            pid: *pid,
            output_path: e.output_path.clone(),
        })
        .collect()
}

/// Register a freshly-spawned bash command so `/ps` can list it and `/stop`
/// can kill its process group. The command keeps running in the foreground
/// (the tool future owns the `Child` and `wait()`s directly); the entry is
/// removed by [`unregister`] when `wait()` returns, or by [`stop`]/[`kill_all`]
/// when the user intervenes.
pub fn register(pid: u32, pgid: libc::pid_t, session_id: String) {
    registry().lock().unwrap().insert(
        pid,
        BgEntry {
            pgid,
            session_id,
            output_path: output_path(pid),
        },
    );
}

/// Remove a registry entry for a command that completed naturally (or whose
/// foreground future otherwise resolved). Idempotent: a no-op if `pid` was
/// already removed by [`stop`]/[`kill_all`].
pub fn unregister(pid: u32) {
    registry().lock().unwrap().remove(&pid);
}

/// Kill the process group of a single registered command by pid and remove its
/// registry entry. Returns `true` if `pid` was registered (and thus
/// signalled), `false` if it was already gone. The `/stop` command currently
/// calls [`kill_all`]; this per-pid variant is exposed for finer control.
pub fn stop(pid: u32) -> bool {
    let entry = registry().lock().unwrap().remove(&pid);
    if let Some(entry) = entry {
        #[cfg(unix)]
        unsafe {
            let _ = libc::kill(-entry.pgid, libc::SIGKILL);
        }
        true
    } else {
        false
    }
}

/// Kill every registered background process group and remove temp files.
/// Returns the number of process groups killed. Used by [`cleanup_all`] at
/// program shutdown and by the display-only `/stop` command
pub fn kill_all() -> usize {
    let entries: Vec<BgEntry> = {
        let mut reg = registry().lock().unwrap();
        reg.drain().map(|(_, e)| e).collect()
    };
    let count = entries.len();
    for entry in entries {
        #[cfg(unix)]
        unsafe {
            let _ = libc::kill(-entry.pgid, libc::SIGKILL);
        }
        let _ = std::fs::remove_file(&entry.output_path);
    }
    count
}

/// Kill every registered background process group and remove temp files.
/// Called at program shutdown.
pub fn cleanup_all() {
    let _ = kill_all();
}

/// Serialize all tests that touch the process-global registry.
///
/// Under parallel test execution a global-draining [`kill_all`] in one
/// test can SIGKILL another test's registered command mid-flight. Holding
/// this shared mutex for the whole duration of every registry-touching
/// test removes that race; it is a `tokio::sync::Mutex` so async tests (e.g. the bash tool tests) can hold it across `.await` too, serializing every registry-touching test.
#[cfg(test)]
pub(crate) fn test_registry_mutex() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}
#[cfg(test)]
mod tests {
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
        st.push_stdout(b"hello");
        st.push_stderr(b"world");
        assert_eq!(&st.stdout_buf, b"hello");
        assert_eq!(&st.stderr_buf, b"world");
        assert!(st.file.is_none());
    }
}

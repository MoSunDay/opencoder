//! Global background-process registry for the bash tool.
//!
//! When a bash command exceeds its timeout, instead of SIGKILL-ing the
//! process group we hand it off to a detached supervisor that keeps the
//! command running in the background, streams its output to a temp file, and
//! cleans up the whole process group when the command exits naturally.

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
    PathBuf::from(format!("/tmp/opencode_bg_{pid}.output"))
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
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            async {
                let _ = stdout_task.await;
                let _ = stderr_task.await;
            },
        )
        .await;

        let code = exit_status
            .ok()
            .and_then(|s| s.code())
            .unwrap_or(-1);

        if let Ok(mut f) = OpenOptions::new().append(true).open(&path) {
            let _ = write!(f, "\n[exit code: {code}]");
        }

        let mut reg = registry().lock().unwrap();
        reg.remove(&pid);
    });
}

/// Kill every registered background process group and remove temp files.
/// Called at program shutdown.
pub fn cleanup_all() {
    let entries: Vec<BgEntry> = {
        let mut reg = registry().lock().unwrap();
        reg.drain().map(|(_, e)| e).collect()
    };
    for entry in entries {
        #[cfg(unix)]
        unsafe {
            let _ = libc::kill(-entry.pgid, libc::SIGKILL);
        }
        let _ = std::fs::remove_file(&entry.output_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_path_format() {
        let p = output_path(12345);
        assert_eq!(p.to_str().unwrap(), "/tmp/opencode_bg_12345.output");
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

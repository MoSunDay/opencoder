//! Global process registry for the bash tool.
//!
//! Every bash command registers itself on spawn so the display-only `/ps`
//! command can list the running process and `/stop` can kill its process
//! group. A bash command runs in the foreground until it exits or until
//! `BASH_TIMEOUT_SECS` elapses (see `tools::bash`); on timeout the still-
//! running command is handed to the detached background supervisor via
//! [`handoff`], which streams output to a temp file the model can read.
//! [`register`] adds the entry, [`unregister`] removes it on completion, and
//! [`stop`]/[`kill_all`] terminate the group on user demand.

use std::collections::{HashMap, VecDeque};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Child;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::bash::process_group::ProcessGroupGuard;

pub const STREAM_OUTPUT_LIMIT_BYTES: usize = 8 * 1024 * 1024;

/// A node may supervise at most this many timed-out bash commands at once.
/// Foreground commands have their own execution slots; this cap covers only
/// commands explicitly handed to detached supervisors.
const MAX_ACTIVE_HANDOFFS: usize = 32;

/// Completed output is temporary diagnostic state. Keep a small recent tail
/// long enough for the model/operator to inspect it, then reclaim it during
/// later background activity. Only paths created and tracked by this process
/// enter this lifecycle; DB rows and user files are never considered.
const MAX_RETAINED_OUTPUTS: usize = 8;
const COMPLETED_OUTPUT_TTL: Duration = Duration::from_secs(60 * 60);

/// Shared capture state for a backgrounded command's stdout/stderr.
///
/// In the foreground phase (`file == None`) it only buffers into `stdout_buf`
/// / `stderr_buf`. After [`handoff`] sets `file`, subsequent `push_*` calls
/// also append to the file so the background output file stays live.
pub struct BgState {
    pub stdout_buf: Vec<u8>,
    pub stderr_buf: Vec<u8>,
    file: Option<std::fs::File>,
    output_limit_error: Option<String>,
    output_limit: CancellationToken,
}

impl BgState {
    pub fn new() -> Self {
        Self {
            stdout_buf: Vec::new(),
            stderr_buf: Vec::new(),
            file: None,
            output_limit_error: None,
            output_limit: CancellationToken::new(),
        }
    }

    /// Append a chunk of stdout. Writes to the file (if handed off) under the
    /// same lock — no await is held.
    pub fn push_stdout(&mut self, data: &[u8]) -> bool {
        self.push(data, true)
    }

    /// Append a chunk of stderr. Same file-write semantics as `push_stdout`.
    pub fn push_stderr(&mut self, data: &[u8]) -> bool {
        self.push(data, false)
    }

    fn push(&mut self, data: &[u8], stdout: bool) -> bool {
        if self.output_limit_error.is_some() {
            return false;
        }
        let (buffer, label) = if stdout {
            (&mut self.stdout_buf, "bash stdout")
        } else {
            (&mut self.stderr_buf, "bash stderr")
        };
        let accepted = data.len().min(STREAM_OUTPUT_LIMIT_BYTES - buffer.len());
        buffer.extend_from_slice(&data[..accepted]);
        if let Some(file) = &mut self.file {
            let _ = file.write_all(&data[..accepted]);
        }
        if accepted == data.len() {
            return true;
        }
        let error =
            format!("output_limit_exceeded: {label} exceeds {STREAM_OUTPUT_LIMIT_BYTES} bytes");
        if let Some(file) = &mut self.file {
            let _ = write!(file, "\n[{error}]\n");
        }
        self.output_limit_error = Some(error);
        self.output_limit.cancel();
        false
    }

    pub fn output_limit_error(&self) -> Option<&str> {
        self.output_limit_error.as_deref()
    }

    pub fn output_limit_token(&self) -> CancellationToken {
        self.output_limit.clone()
    }
}

impl Default for BgState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

/// Drain one tool pipe into the bounded shared capture. An overflow kills the
/// command's private process group so a blocked producer cannot linger.
pub async fn drain_output<R>(
    mut pipe: R,
    state: std::sync::Arc<Mutex<BgState>>,
    stream: OutputStream,
    pgid: libc::pid_t,
) where
    R: AsyncRead + Unpin,
{
    let mut chunk = [0u8; 8192];
    loop {
        let count = match pipe.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(count) => count,
        };
        let accepted = {
            let mut state = state.lock().unwrap();
            match stream {
                OutputStream::Stdout => state.push_stdout(&chunk[..count]),
                OutputStream::Stderr => state.push_stderr(&chunk[..count]),
            }
        };
        if !accepted {
            #[cfg(unix)]
            unsafe {
                let _ = libc::kill(-pgid, libc::SIGKILL);
            }
            return;
        }
    }
}

/// Path of the background output file for a given pid.
pub fn output_path(pid: u32) -> PathBuf {
    PathBuf::from(format!("/tmp/opencoder_bg_{pid}.output"))
}

struct BgEntry {
    pgid: libc::pid_t,
    supervisor: Option<crate::process::SignalTarget>,
    #[allow(dead_code)]
    session_id: String,
    output_path: PathBuf,
}

fn registry() -> &'static Mutex<HashMap<u32, BgEntry>> {
    static REG: OnceLock<Mutex<HashMap<u32, BgEntry>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

struct SupervisorTask {
    abort: Option<tokio::task::AbortHandle>,
    output_path: PathBuf,
}

fn supervisor_tasks() -> &'static Mutex<HashMap<u64, SupervisorTask>> {
    static TASKS: OnceLock<Mutex<HashMap<u64, SupervisorTask>>> = OnceLock::new();
    TASKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_supervisor_id() -> u64 {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

struct CompletedOutput {
    path: PathBuf,
    completed_at: Instant,
}

#[derive(Default)]
struct CompletedOutputs {
    cleaning: bool,
    entries: VecDeque<CompletedOutput>,
}

fn completed_outputs() -> &'static Mutex<CompletedOutputs> {
    static OUTPUTS: OnceLock<Mutex<CompletedOutputs>> = OnceLock::new();
    OUTPUTS.get_or_init(|| Mutex::new(CompletedOutputs::default()))
}

fn completed_output_wakeup() -> &'static tokio::sync::Notify {
    static WAKEUP: OnceLock<tokio::sync::Notify> = OnceLock::new();
    WAKEUP.get_or_init(tokio::sync::Notify::new)
}

fn completed_output_sweeper() -> &'static Mutex<Option<tokio::task::AbortHandle>> {
    static SWEEPER: OnceLock<Mutex<Option<tokio::task::AbortHandle>>> = OnceLock::new();
    SWEEPER.get_or_init(|| Mutex::new(None))
}

/// Hand a timed-out command to a detached background supervisor.
///
/// After reserving one of 32 supervisor slots, opens/truncates the output file,
/// flushes the already-captured stdout/stderr buffers to it, sets `state.file`
/// so subsequent incremental pushes go straight to the file, and spawns a
/// detached task that owns the foreground process-group guard and:
///
/// 1. Waits for the child to exit naturally (kill_on_drop is defanged because
///    we own the `Child` and only drop it after `wait()`).
/// 2. Awaits both drain tasks until EOF so the file captures the full output.
/// 3. Appends `[exit code: N]`.
/// 4. terminates lingering process-group members and removes the live entry;
/// 5. retains the file for at most one hour and among the eight newest files;
/// 6. removes its own supervisor handle.
#[allow(clippy::too_many_arguments)]
pub(super) async fn handoff(
    pid: u32,
    child: Child,
    stdout_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
    state: std::sync::Arc<Mutex<BgState>>,
    process_group: ProcessGroupGuard,
) -> Result<(), String> {
    handoff_with_limit(
        pid,
        child,
        stdout_task,
        stderr_task,
        state,
        process_group,
        MAX_ACTIVE_HANDOFFS,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn handoff_with_limit(
    pid: u32,
    mut child: Child,
    stdout_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
    state: std::sync::Arc<Mutex<BgState>>,
    mut process_group: ProcessGroupGuard,
    active_limit: usize,
) -> Result<(), String> {
    begin_background_lifecycle();
    prune_completed_outputs(Instant::now());
    let path = output_path(pid);
    let supervisor_id = next_supervisor_id();
    let reserved = {
        let mut tasks = supervisor_tasks().lock().unwrap();
        if tasks.len() >= active_limit {
            false
        } else {
            tasks.insert(
                supervisor_id,
                SupervisorTask {
                    abort: None,
                    output_path: path.clone(),
                },
            );
            true
        }
    };
    if !reserved {
        cleanup_unaccepted(child, stdout_task, stderr_task, process_group).await;
        return Err(format!(
            "background_process_limit_exceeded: at most {active_limit} handed-off bash commands may run"
        ));
    }

    // Open/truncate + flush captured buffers + activate file mode.
    let opened = (|| {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .map_err(|error| format!("cannot create background output: {error}"))?;
        let mut st = state.lock().unwrap();
        file.write_all(&st.stdout_buf)
            .map_err(|error| format!("cannot write background output: {error}"))?;
        if !st.stderr_buf.is_empty() {
            file.write_all(b"\n[stderr]\n")
                .and_then(|()| file.write_all(&st.stderr_buf))
                .map_err(|error| format!("cannot write background output: {error}"))?;
        }
        if let Some(error) = st.output_limit_error() {
            writeln!(file, "\n[{error}]")
                .map_err(|error| format!("cannot write background output: {error}"))?;
        }
        st.file = Some(file);
        Ok::<(), String>(())
    })();
    if let Err(error) = opened {
        supervisor_tasks().lock().unwrap().remove(&supervisor_id);
        cleanup_unaccepted(child, stdout_task, stderr_task, process_group).await;
        let _ = std::fs::remove_file(path);
        return Err(error);
    }

    // The task cannot run before its AbortHandle is registered: this start
    // gate prevents a short command from completing first and leaving a stale
    // handle in the global map.
    let (start_tx, start_rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        if start_rx.await.is_err() {
            return;
        }
        let exit_status = child.wait().await;

        // Terminate lingering descendants and unregister before draining, so
        // inherited pipe writers cannot keep the drain tasks alive.
        process_group.terminate();

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
        drop(state.lock().unwrap().file.take());
        retain_completed_output(path, Instant::now());
        supervisor_tasks().lock().unwrap().remove(&supervisor_id);
    });
    let mut tasks = supervisor_tasks().lock().unwrap();
    let Some(task) = tasks.get_mut(&supervisor_id) else {
        handle.abort();
        return Err("background supervisor rejected during shutdown".into());
    };
    task.abort = Some(handle.abort_handle());
    drop(tasks);
    let _ = start_tx.send(());
    Ok(())
}

async fn cleanup_unaccepted(
    mut child: Child,
    stdout_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
    mut process_group: ProcessGroupGuard,
) {
    let supervised = process_group.is_supervised();
    process_group.terminate();
    let _ = crate::process::wait_owned_child(&mut child, supervised).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        let _ = stdout_task.await;
        let _ = stderr_task.await;
    })
    .await;
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
    prune_completed_outputs(Instant::now());
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
            supervisor: None,
            session_id,
            output_path: output_path(pid),
        },
    );
}

pub(crate) fn register_supervised(
    pid: u32,
    supervisor: crate::process::SignalTarget,
    session_id: String,
) {
    registry().lock().unwrap().insert(
        pid,
        BgEntry {
            pgid: pid as libc::pid_t,
            supervisor: Some(supervisor),
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

pub(crate) fn unregister_group(pid: u32, pgid: libc::pid_t) {
    let mut registry = registry().lock().unwrap();
    if registry.get(&pid).is_some_and(|entry| entry.pgid == pgid) {
        registry.remove(&pid);
    }
}

/// Kill the process group of a single registered command by pid and remove its
/// registry entry. Returns `true` if `pid` was registered (and thus
/// signalled), `false` if it was already gone. The `/stop` command currently
/// calls [`kill_all`]; this per-pid variant is exposed for finer control.
pub fn stop(pid: u32) -> bool {
    let entry = registry().lock().unwrap().remove(&pid);
    if let Some(entry) = entry {
        terminate_entry(&entry);
        true
    } else {
        false
    }
}

/// Kill every registered process group. Background output remains readable
/// until the completed-output TTL/count lifecycle reclaims it.
/// Returns the number of process groups killed. Used by [`cleanup_all`] at
/// program shutdown and by the display-only `/stop` command
pub fn kill_all() -> usize {
    let entries: Vec<BgEntry> = {
        let mut reg = registry().lock().unwrap();
        reg.drain().map(|(_, e)| e).collect()
    };
    let count = entries.len();
    for entry in entries {
        terminate_entry(&entry);
    }
    count
}

/// Kill tracked process groups, abort supervisors, and delete only temporary
/// output paths created and tracked by this process. This is the production
/// shutdown hook; it never walks directories or touches DB/user artifacts.
pub fn cleanup_all() {
    let active_entries: Vec<BgEntry> = {
        let mut reg = registry().lock().unwrap();
        reg.drain().map(|(_, entry)| entry).collect()
    };
    for entry in &active_entries {
        terminate_entry(entry);
    }
    let tasks: Vec<SupervisorTask> = supervisor_tasks()
        .lock()
        .unwrap()
        .drain()
        .map(|(_, task)| task)
        .collect();
    let completed = {
        let mut state = completed_outputs().lock().unwrap();
        state.cleaning = true;
        state
            .entries
            .drain(..)
            .map(|entry| entry.path)
            .collect::<Vec<_>>()
    };
    for task in &tasks {
        if let Some(abort) = &task.abort {
            abort.abort();
        }
    }
    if let Some(sweeper) = completed_output_sweeper().lock().unwrap().take() {
        sweeper.abort();
    }
    let paths = tasks
        .into_iter()
        .map(|task| task.output_path)
        .chain(completed);
    remove_temp_outputs(paths);
}

fn terminate_entry(entry: &BgEntry) {
    if let Some(supervisor) = &entry.supervisor {
        let _ = supervisor.signal(libc::SIGTERM);
    } else {
        #[cfg(unix)]
        unsafe {
            let _ = libc::kill(-entry.pgid, libc::SIGKILL);
        }
    }
}

/// `cleanup_all` is terminal in production. Tests may start a fresh lifecycle
/// in the same process; a new accepted handoff is the explicit reset point.
fn begin_background_lifecycle() {
    completed_outputs().lock().unwrap().cleaning = false;
}

fn retain_completed_output(path: PathBuf, now: Instant) {
    let remove = {
        let mut state = completed_outputs().lock().unwrap();
        if state.cleaning {
            vec![path]
        } else {
            state.entries.retain(|entry| entry.path != path);
            state.entries.push_back(CompletedOutput {
                path,
                completed_at: now,
            });
            prunable_completed_outputs(&mut state.entries, now)
        }
    };
    remove_temp_outputs(remove);
    ensure_completed_output_sweeper();
    completed_output_wakeup().notify_one();
}

fn prune_completed_outputs(now: Instant) {
    let remove = {
        let mut state = completed_outputs().lock().unwrap();
        prunable_completed_outputs(&mut state.entries, now)
    };
    remove_temp_outputs(remove);
}

/// Maintain exactly one lazy expiry task per runtime. Count pruning is
/// synchronous; this task enforces the one-hour TTL even on an otherwise idle
/// long-running node. A test without a Tokio runtime simply uses explicit
/// pruning, while production supervisors always have a runtime available.
fn ensure_completed_output_sweeper() {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let mut slot = completed_output_sweeper().lock().unwrap();
    if slot.as_ref().is_some_and(|task| !task.is_finished()) {
        return;
    }
    let task = runtime.spawn(async {
        loop {
            let deadline = completed_outputs()
                .lock()
                .unwrap()
                .entries
                .front()
                .map(|entry| entry.completed_at + COMPLETED_OUTPUT_TTL);
            match deadline {
                Some(deadline) => {
                    tokio::select! {
                        _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
                            prune_completed_outputs(Instant::now());
                        }
                        _ = completed_output_wakeup().notified() => {}
                    }
                }
                None => completed_output_wakeup().notified().await,
            }
        }
    });
    *slot = Some(task.abort_handle());
}

/// Select expired/over-capacity entries oldest first. Filesystem deletion is
/// deliberately outside this pure collection transform and outside the lock.
fn prunable_completed_outputs(
    entries: &mut VecDeque<CompletedOutput>,
    now: Instant,
) -> Vec<PathBuf> {
    let mut remove = Vec::new();
    while entries.front().is_some_and(|entry| {
        now.saturating_duration_since(entry.completed_at) >= COMPLETED_OUTPUT_TTL
    }) {
        if let Some(entry) = entries.pop_front() {
            remove.push(entry.path);
        }
    }
    while entries.len() > MAX_RETAINED_OUTPUTS {
        if let Some(entry) = entries.pop_front() {
            remove.push(entry.path);
        }
    }
    remove
}

fn remove_temp_outputs(paths: impl IntoIterator<Item = PathBuf>) {
    let mut unique = std::collections::HashSet::new();
    for path in paths {
        if unique.insert(path.clone()) {
            let _ = std::fs::remove_file(path);
        }
    }
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
pub(crate) fn all_task_handles_len_for_test() -> usize {
    supervisor_tasks().lock().unwrap().len()
}
#[cfg(test)]
pub(crate) fn retained_outputs_len_for_test() -> usize {
    completed_outputs().lock().unwrap().entries.len()
}

#[cfg(test)]
#[path = "bash/background_tests.rs"]
mod tests;

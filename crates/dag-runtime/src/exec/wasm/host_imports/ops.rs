//! Op registry execution: spawn a whitelisted `dag.ops` command,
//! argv-only with `env_keys`-declared env, bounded capture, evidence
//! log, timeout / overflow / cancel kills.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use opencoder_core::config::DagOpConfig;

use crate::step_log::{StepOutputLog, Stream};

use super::HostState;

/// Default op wall-clock budget when the registration sets no timeout.
const DEFAULT_OP_TIMEOUT_SECS: u64 = 600;
/// Poll cadence of the op reaper loop.
const REAP_POLL_MS: u64 = 50;

/// Cap on captured op stdout+stderr (bytes, per op run). Overflow kills
/// the child and errors the step — an unbounded pipe can never stall the
/// node or blow the artifact store.
pub(crate) const OP_LOG_LIMIT_BYTES: usize = 256 * 1024;

/// `op_id` is also the evidence filename: alphanumerics, `-` and `_`
/// only, bounded length.
pub(crate) fn validate_op_id(op_id: &str) -> Result<()> {
    let ok = !op_id.is_empty()
        && op_id.len() <= 64
        && op_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(anyhow!(
            "malformed op id {op_id:?}: [A-Za-z0-9_-]{{1,64}} expected"
        ))
    }
}

/// Why the reaper killed (or decided to kill) the op child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KillReason {
    Overflow,
    Timeout,
    Cancel,
}

impl KillReason {
    fn message(self, op_id: &str, detail: &str) -> String {
        match self {
            KillReason::Overflow => {
                format!("op `{op_id}` output exceeds {OP_LOG_LIMIT_BYTES} bytes (limit kill)")
            }
            KillReason::Timeout => format!("op `{op_id}` timeout after {detail}s (killed)"),
            KillReason::Cancel => format!("op `{op_id}` killed: step cancelled"),
        }
    }
}

/// Run one registered op synchronously (the guest thread is already a
/// `spawn_blocking` worker). Returns the child exit code; host-side
/// failures return `Err` → the guest traps → the step errors.
pub(crate) fn run_op(state: &mut HostState, op_id: &str, args: &str) -> Result<i32> {
    validate_op_id(op_id)?;
    let Some(cfg) = state.ops.get(op_id).cloned() else {
        return Err(anyhow!(
            "unknown op `{op_id}`: not registered in dag.ops (fail-closed)"
        ));
    };
    if cfg.termination_grace_secs > 900 {
        return Err(anyhow!(
            "op `{op_id}` termination grace exceeds 900 seconds"
        ));
    }
    let mut argv: Vec<String> = cfg.command.split_whitespace().map(str::to_string).collect();
    argv.extend(args.split_whitespace().map(str::to_string));
    let Some(program) = argv.first().cloned() else {
        return Err(anyhow!("op `{op_id}` has an empty command"));
    };

    let mut command = Command::new(program.clone());
    command
        .args(&argv[1..])
        .current_dir(&state.run_root)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Own process group: a timeout/cancel kill must take the whole tree
    // down (shell wrappers fork children that inherit the pipes and would
    // otherwise stall the bounded readers for the full timeout).
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    for key in &cfg.env_keys {
        if let Ok(value) = std::env::var(key) {
            command.env(key, value);
        }
    }
    let mirror = state.output.clone();
    if let Some(log) = &mirror {
        log.push_bytes(
            Stream::Stdout,
            format!("[op {op_id}] start: {program}\n").as_bytes(),
        );
    }
    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|e| anyhow!("spawn op `{op_id}` ({program}): {e}"))?;

    // Bounded, mirrored capture shared by the two pipe readers.
    let capture = Arc::new(OpCapture::default());
    let out_reader = spawn_pipe_reader(
        child.stdout.take(),
        Arc::clone(&capture),
        mirror.clone(),
        Stream::Stdout,
    );
    let err_reader = spawn_pipe_reader(
        child.stderr.take(),
        Arc::clone(&capture),
        mirror.clone(),
        Stream::Stderr,
    );

    let budget = Duration::from_secs(cfg.timeout_secs.unwrap_or(DEFAULT_OP_TIMEOUT_SECS));
    let deadline = Instant::now() + budget;
    let mut exit = reap(
        &mut child,
        &capture,
        &state.cancel,
        deadline,
        Duration::from_secs(cfg.termination_grace_secs),
    );
    let _ = out_reader.join();
    let _ = err_reader.join();
    // A fast exit can beat the reaper's overflow poll (and the readers
    // only finish after the pipes close): re-check once they are drained.
    if capture.overflow() {
        exit = Err(KillReason::Overflow);
    }
    let captured = capture.snapshot();

    // Evidence ALWAYS lands, even for a killed op: the partial output is
    // exactly what forensics needs.
    write_op_log(
        state,
        op_id,
        &exit,
        started,
        &cfg,
        &captured,
        capture.overflow(),
    )?;
    if let Some(log) = &state.output {
        log.push_bytes(
            Stream::Stdout,
            format!("[op {op_id}] end: {}\n", exit_label(&exit)).as_bytes(),
        );
    }
    match exit {
        Ok(code) => Ok(code),
        Err(reason) => Err(anyhow!(
            "{}",
            reason.message(
                op_id,
                &cfg.timeout_secs
                    .unwrap_or(DEFAULT_OP_TIMEOUT_SECS)
                    .to_string()
            )
        )),
    }
}

/// Human label for the op end marker (log mirror + evidence header).
fn exit_label(exit: &Result<i32, KillReason>) -> String {
    match exit {
        Ok(code) => format!("exit {code}"),
        Err(reason) => format!("killed ({reason:?})"),
    }
}

/// Kill the op child's whole process group (the child was spawned with
/// `process_group(0)`), falling back to the direct child.
fn kill_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        let pgid = child.id() as i32;
        if unsafe { libc::kill(-pgid, libc::SIGKILL) } == 0 {
            return;
        }
    }
    let _ = child.kill();
}

/// Wait for the child, checking (in order) output overflow, step
/// cancellation and the wall-clock deadline. `Ok(exit_code)` on a natural
/// exit; `Err(reason)` after the corresponding kill.
fn reap(
    child: &mut Child,
    capture: &OpCapture,
    cancel: &AtomicBool,
    deadline: Instant,
    grace: Duration,
) -> Result<i32, KillReason> {
    let mut terminating: Option<(KillReason, Instant)> = None;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            if terminating.is_some() {
                // Cleanup may exit while descendants still hold the captured pipes.
                kill_tree(child);
            }
            return terminating.map_or(Ok(status.code().unwrap_or(-1)), |(reason, _)| Err(reason));
        }
        let reason = if capture.overflow() {
            Some(KillReason::Overflow)
        } else if cancel.load(Ordering::SeqCst) {
            Some(KillReason::Cancel)
        } else if Instant::now() >= deadline {
            Some(KillReason::Timeout)
        } else {
            None
        };
        if let Some(reason) = reason {
            if reason != KillReason::Overflow && !grace.is_zero() {
                let (_, sent_at) = terminating.get_or_insert_with(|| {
                    #[cfg(unix)]
                    unsafe {
                        libc::kill(child.id() as i32, libc::SIGTERM);
                    }
                    (reason, Instant::now())
                });
                if sent_at.elapsed() < grace {
                    std::thread::sleep(Duration::from_millis(REAP_POLL_MS));
                    continue;
                }
            }
            kill_tree(child);
            let _ = child.wait();
            // A killed child still has a wait status; report the reason.
            return Err(reason);
        }
        std::thread::sleep(Duration::from_millis(REAP_POLL_MS));
    }
}

/// Persist the op evidence file `<step_dir>/ops/<op_id>.log`.
fn write_op_log(
    state: &HostState,
    op_id: &str,
    exit: &Result<i32, KillReason>,
    started: Instant,
    cfg: &DagOpConfig,
    captured: &[u8],
    overflow: bool,
) -> Result<()> {
    let dir = state.step_dir.join("ops");
    std::fs::create_dir_all(&dir).map_err(|e| anyhow!("op log dir {}: {e}", dir.display()))?;
    let mut body = String::new();
    let head = match exit {
        Ok(code) => format!("# op {op_id} exit={code}"),
        Err(reason) => format!("# op {op_id} killed ({reason:?})"),
    };
    body.push_str(&format!(
        "{head} duration_ms={} timeout_secs={} truncated={}\n",
        started.elapsed().as_millis(),
        cfg.timeout_secs.unwrap_or(DEFAULT_OP_TIMEOUT_SECS),
        overflow
    ));
    let text = String::from_utf8_lossy(&captured[..captured.len().min(OP_LOG_LIMIT_BYTES)]);
    body.push_str("-- output --\n");
    body.push_str(&text);
    if !text.ends_with('\n') {
        body.push('\n');
    }
    std::fs::write(dir.join(format!("{op_id}.log")), body)
        .map_err(|e| anyhow!("write op log for `{op_id}`: {e}"))
}

/// Byte-bounded shared capture for one op's pipes.
#[derive(Default)]
pub(crate) struct OpCapture {
    buf: Mutex<Vec<u8>>,
    overflow: AtomicBool,
}

impl OpCapture {
    fn push(&self, bytes: &[u8]) {
        if let Ok(mut buf) = self.buf.lock() {
            if buf.len() + bytes.len() <= OP_LOG_LIMIT_BYTES {
                buf.extend_from_slice(bytes);
            } else {
                self.overflow.store(true, Ordering::SeqCst);
            }
        }
    }

    fn overflow(&self) -> bool {
        self.overflow.load(Ordering::SeqCst)
    }

    fn snapshot(&self) -> Vec<u8> {
        self.buf.lock().map(|b| b.clone()).unwrap_or_default()
    }
}

/// Drain one child pipe into the shared capture (plus the live mirror).
fn spawn_pipe_reader<R: Read + Send + 'static>(
    pipe: Option<R>,
    capture: Arc<OpCapture>,
    mirror: Option<StepOutputLog>,
    stream: Stream,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let Some(mut pipe) = pipe else { return };
        let mut buf = [0u8; 4096];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    if let Some(log) = &mirror {
                        log.push_bytes(stream, &buf[..n]);
                    }
                    capture.push(&buf[..n]);
                }
            }
        }
    })
}

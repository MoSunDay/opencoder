//! Process-wide termination-signal guard for the captured terminal.
//!
//! `TerminalGuard::enter()` enables raw mode + alt-screen + mouse capture +
//! bracketed paste. If the process dies from a termination signal before
//! `TerminalGuard::drop` runs, the host terminal keeps **mouse reporting
//! enabled** — after exit every click or drag prints escape garbage
//! (`M#…`/`[M…`) into the shell. The liveness supervisor covers this too, but
//! only from the moment it is spawned (inside `run_app` / onboarding), leaving
//! a boot window — terminal captured, supervisor not yet armed — where a
//! `SIGTERM`/`SIGHUP` (tmux kill-pane, ssh drop) bricked the terminal.
//!
//! This guard is armed by `TerminalGuard::enter()` itself, so signal-driven
//! restoration is live from the first millisecond the terminal is captured,
//! on every path (onboarding wizard, chat loop, shutdown). It is a
//! **process-wide singleton** (latched once): the liveness supervisor no
//! longer registers signals itself, keeping a single writer for the restore
//! sequences (two threads racing `restore()` could interleave identical
//! escape sequences into garbage).

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use signal_hook::consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
use signal_hook::flag;

use crate::terminal::TerminalGuard;

/// Poll cadence for the watcher thread. Well under human perception; the
/// thread is parked in `sleep` and costs nothing when idle.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Latch: the guard is armed at most once per process.
static ARMED: AtomicBool = AtomicBool::new(false);

/// Signals that mean "terminate now": restore the terminal before the default
/// disposition would kill the process with modes still set. (`SIGINT` never
/// fires from Ctrl+C while raw mode is on — ISIG is disabled — but covers
/// externally sent `kill -INT`.)
const SIGNALS: [(i32, &str); 4] = [
    (SIGHUP, "SIGHUP"),
    (SIGINT, "SIGINT"),
    (SIGQUIT, "SIGQUIT"),
    (SIGTERM, "SIGTERM"),
];

/// Arm the process-wide signal guard (idempotent). Called by
/// `TerminalGuard::enter()`; safe to call again — later calls are no-ops.
pub(crate) fn arm_once() {
    if try_arm(&ARMED) {
        spawn_watcher();
    }
}

/// Pure latch step, factored out so the once-only guarantee is unit-testable
/// without spawning the real watcher thread (whose signal registrations would
/// alter this process's own signal dispositions).
fn try_arm(latched: &AtomicBool) -> bool {
    latched
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

/// Best-effort signal → flag registration. Returns `None` on failure; the
/// TerminalGuard Drop/panic-hook paths still restore on non-signal exits.
fn watch_signal(signum: i32) -> Option<Arc<AtomicBool>> {
    let flag = Arc::new(AtomicBool::new(false));
    match flag::register(signum, Arc::clone(&flag)) {
        Ok(_) => Some(flag),
        Err(_) => None,
    }
}

/// The watcher body. Sleep-poll every flag; on the first trip restore the
/// terminal, tell the user why the process is going away, and exit cleanly
/// (the session is persisted on every write — `opencoder --continue` resumes).
fn spawn_watcher() {
    let watched: Vec<(Arc<AtomicBool>, &'static str)> = SIGNALS
        .iter()
        .filter_map(|&(sig, name)| watch_signal(sig).map(|f| (f, name)))
        .collect();
    if watched.is_empty() {
        tracing::warn!("signal guard: no signals registered, running unguarded");
        return;
    }
    thread::spawn(move || loop {
        thread::sleep(POLL_INTERVAL);
        let Some(name) = watched
            .iter()
            .find(|(f, _)| f.load(Ordering::Relaxed))
            .map(|(_, name)| *name)
        else {
            continue;
        };
        // Restore FIRST: while still in alt-screen + raw mode any stderr
        // write lands as raw escape garbage overlaying the interface.
        TerminalGuard::restore();
        let _ = writeln!(std::io::stderr(), "{}", exit_message(name));
        std::process::exit(0);
    });
}

/// The stderr line printed after the terminal is restored. Pure so the
/// user-facing copy (binary name, signal name, `--continue` hint) stays
/// unit-testable without threads or a real signal.
fn exit_message(signal_name: &str) -> String {
    format!(
        "opencoder: received {signal_name} — terminal restored, exiting. \
         Reopen with `opencoder --continue` to resume this session."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The latch must arm exactly once: the first CAS wins, every later call
    /// is a no-op. This is what keeps a single watcher thread (single writer
    /// for the restore sequences) per process.
    #[test]
    fn try_arm_latches_once() {
        let latch = AtomicBool::new(false);
        assert!(try_arm(&latch), "first arm must win");
        assert!(!try_arm(&latch), "second arm must be a no-op");
        assert!(!try_arm(&latch), "later arms must stay no-ops");
    }

    /// Copy contract: binary name, signal name, resume hint — and no stale
    /// bare `opencode` name (plain `contains` would lie: `opencoder` contains
    /// `opencode`).
    #[test]
    fn exit_message_names_signal_and_resume_hint() {
        let msg = exit_message("SIGTERM");
        assert!(
            msg.starts_with("opencoder:"),
            "must lead with binary name: {msg}"
        );
        assert!(msg.contains("SIGTERM"), "must name the signal: {msg}");
        assert!(
            msg.contains("opencoder --continue"),
            "must advertise the resume hint: {msg}"
        );
        assert!(
            !msg.contains("opencode ") && !msg.contains("opencode:"),
            "stale bare `opencode` name in copy: {msg}"
        );
    }
}

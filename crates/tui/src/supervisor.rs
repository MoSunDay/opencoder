//! Liveness supervisor for the TUI input collector.
//!
//! Prevents the "left it idle and it froze — even Ctrl+C/D can't quit, had to
//! `kill -9`" failure. Two root causes are neutralised:
//!
//! 1. **crossterm mio busy-loop on tty death.** When the pty master closes
//!    (SSH disconnect, `tmux kill-pane`, terminal process killed, lid close on
//!    a remote session) the slave read starts returning `Ok(0)`/`EIO`.
//!    crossterm 0.28's `UnixInternalEventSource::try_read` treats *neither* as
//!    a break condition — it spins forever in a tight user-space loop while
//!    *holding the global `INTERNAL_EVENT_READER` mutex*. Our bounded
//!    `event::poll(150ms)` therefore never returns; the collector thread stops
//!    bumping its heartbeat; no key ever reaches the main loop. Process alive,
//!    screen static, all keys dead — exactly the report.
//!
//! 2. ~~Termination signals~~ — moved to [`crate::signal_guard`], which is
//!    armed by `TerminalGuard::enter()` itself and therefore covers the whole
//!    captured-terminal lifetime (including the boot window *before* this
//!    supervisor is spawned). The supervisor no longer registers signals:
//!    exactly one writer must own the restore sequences, or two racing
//!    `restore()` calls could interleave identical escape sequences into
//!    garbage.
//!
//! The supervisor runs on a **dedicated OS thread** (immune to tokio runtime
//! starvation from the busy-loop burning a core) and, on a wedge, restores the
//! terminal and exits cleanly. Sessions are persisted to the `Store` on every
//! write, so the user restarts and `--continue`s instead of staring at a
//! frozen screen.
//!
//! Why not "recover and stay alive"? Once crossterm wedges it holds the one
//! global event mutex; a fresh collector thread cannot read events either, and
//! a dead tty yields no input anyway. The only correct action is a clean exit.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::terminal::TerminalGuard;

/// A wedged collector is declared once the heartbeat is this stale. The pump
/// bumps every ≤150 ms, so this is a ~33× margin — immune to scheduling jitter
/// and to system suspend (`Instant` is `CLOCK_MONOTONIC`, which does not
/// advance while suspended, so a suspend/resume cycle never trips it).
pub(crate) const WEDGE_TIMEOUT: Duration = Duration::from_secs(5);

/// Poll cadence of the supervisor thread.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Pump liveness probe. Bumped by the input collector at the top of every poll
/// cycle; read by the supervisor. Monotonic-ms since the shared epoch keeps it
/// independent of wall-clock skew and suspend.
#[derive(Clone)]
pub struct Heartbeat {
    last_ms: Arc<AtomicU64>,
    epoch: Instant,
}

impl Heartbeat {
    pub fn new() -> Self {
        let hb = Self {
            last_ms: Arc::new(AtomicU64::new(0)),
            epoch: Instant::now(),
        };
        hb.bump();
        hb
    }

    /// Record "alive now". Called by the pump thread each iteration, *before*
    /// the blocking `event::poll` — so a poll that never returns stops bumping.
    pub(crate) fn bump(&self) {
        let ms = self.epoch.elapsed().as_millis() as u64;
        self.last_ms.store(ms, Ordering::Relaxed);
    }

    pub(crate) fn last_ms(&self) -> u64 {
        self.last_ms.load(Ordering::Relaxed)
    }

    pub(crate) fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }
}

impl Default for Heartbeat {
    fn default() -> Self {
        Self::new()
    }
}

/// Pure decision: is the input collector wedged?
///
/// Factored out (no threads, no `process::exit`) so the exact thresholds are
/// unit-testable. `active` is flipped false by the main loop when it begins a
/// normal shutdown — the heartbeat stalls once the pump is dropped, which is
/// expected then, not a wedge. Signals are NOT handled here: they belong to
/// [`crate::signal_guard`] (armed from `TerminalGuard::enter()`).
pub(crate) fn is_wedged(now_ms: u64, last_alive_ms: u64, active: bool, wedge_ms: u64) -> bool {
    active && now_ms.saturating_sub(last_alive_ms) > wedge_ms
}

/// The stderr message printed after the terminal is restored, just before the
/// supervisor exits the process. Pure so the user-facing copy (binary name,
/// `--continue` hint) is unit-testable without spawning threads.
pub(crate) fn exit_message() -> String {
    "opencoder: input collector InputWedge — terminal restored, exiting. Reopen with `opencoder --continue` to resume this session."
        .to_string()
}

/// Spawn the supervisor thread. On a trip it restores the terminal and exits the
/// process (the session is already persisted, so the user can `--continue`).
///
/// Polls every [`POLL_INTERVAL`]; on a healthy system the decision
/// ([`is_wedged`]) stays false forever. The thread is detached — it dies
/// with the process on a normal exit, and is the thing that *makes* the process
/// exit on an abnormal one.
pub(crate) fn spawn(heartbeat: Heartbeat, active: Arc<AtomicBool>) {
    let wedge_ms = WEDGE_TIMEOUT.as_millis() as u64;
    thread::spawn(move || loop {
        thread::sleep(POLL_INTERVAL);
        if is_wedged(
            heartbeat.now_ms(),
            heartbeat.last_ms(),
            active.load(Ordering::Relaxed),
            wedge_ms,
        ) {
            // Restore the terminal FIRST: while still in alt-screen + raw mode
            // any stderr write lands as raw escape garbage overlaying the
            // interface. Once `restore()` has left the alternate screen stderr
            // is safely visible (or harmlessly discarded if the tty is gone).
            TerminalGuard::restore();
            let _ = writeln!(std::io::stderr(), "{}", exit_message());
            std::process::exit(0);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_message_leads_with_opencoder_and_resume_hint() {
        let msg = exit_message();
        assert!(
            msg.starts_with("opencoder:"),
            "must lead with binary name: {msg}"
        );
        assert!(
            msg.contains("opencoder --continue"),
            "must advertise the resume hint: {msg}"
        );
        assert!(
            msg.contains("InputWedge"),
            "must name the trip reason: {msg}"
        );
        // Word-boundary check: the old bare `opencode` name must be gone.
        // (`opencoder` contains `opencode`, so plain contains() would lie.)
        assert!(
            !msg.contains("opencode ") && !msg.contains("opencode:"),
            "stale bare `opencode` name in copy: {msg}"
        );
    }

    #[test]
    fn heartbeat_advances_on_bump() {
        let hb = Heartbeat::new();
        let a = hb.last_ms();
        // Monotonic: a later bump records a >= timestamp.
        std::thread::sleep(Duration::from_millis(5));
        hb.bump();
        let b = hb.last_ms();
        assert!(b >= a, "heartbeat must not go backwards: {a} -> {b}");
        assert!(hb.now_ms() >= b, "now must be >= last bump");
    }

    #[test]
    fn is_wedged_false_when_fresh_and_quiet() {
        let now = 10_000;
        let last = 9_990; // 10 ms stale — far under the 5 s wedge window
        assert!(!is_wedged(now, last, true, 5_000));
    }

    #[test]
    fn is_wedged_true_when_stale_and_active() {
        let now = 10_000;
        let last = 4_000; // 6 s stale > 5 s
        assert!(is_wedged(now, last, true, 5_000));
    }

    #[test]
    fn is_wedged_ignores_staleness_during_shutdown() {
        // `active = false` (normal shutdown, pump dropped) → never a wedge.
        assert!(!is_wedged(10_000, 0, false, 5_000));
    }

    #[test]
    fn is_wedged_boundary_is_strictly_greater() {
        // Exactly at the wedge window is NOT a trip (off-by-one safety).
        assert!(!is_wedged(5_000, 0, true, 5_000));
        assert!(is_wedged(5_001, 0, true, 5_000));
    }
}

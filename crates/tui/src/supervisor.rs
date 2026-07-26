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
//! 2. **Termination signals** (`SIGHUP`/`SIGTERM`/…) that default-terminate the
//!    process *without* running `TerminalGuard::drop`, leaving the terminal in
//!    raw mode + alt-screen (a "bricked" terminal needing `reset`).
//!
//! The supervisor runs on a **dedicated OS thread** (immune to tokio runtime
//! starvation from the busy-loop burning a core) and, on either condition,
//! restores the terminal and exits cleanly. Sessions are persisted to the
//! `Store` on every write, so the user restarts and `--continue`s instead of
//! staring at a frozen screen.
//!
//! Why not "recover and stay alive"? Once crossterm wedges it holds the one
//! global event mutex; a fresh collector thread cannot read events either, and
//! a dead tty yields no input anyway. The only correct action is a clean exit.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use signal_hook::consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
use signal_hook::flag;

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

/// Why the supervisor tripped.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum Trip {
    Signal,
    InputWedge,
}

/// Pure decision: should the supervisor restore+exit?
///
/// Factored out (no threads, no `process::exit`) so the exact thresholds are
/// unit-testable. `active` is flipped false by the main loop when it begins a
/// normal shutdown — the heartbeat stalls once the pump is dropped, which is
/// expected then, not a wedge. Signals are honoured regardless of `active`.
pub(crate) fn trip_reason(
    now_ms: u64,
    last_alive_ms: u64,
    any_signal: bool,
    active: bool,
    wedge_ms: u64,
) -> Option<Trip> {
    if any_signal {
        return Some(Trip::Signal);
    }
    if active && now_ms.saturating_sub(last_alive_ms) > wedge_ms {
        return Some(Trip::InputWedge);
    }
    None
}

/// Best-effort signal → flag registration. Returns `None` on failure; the
/// heartbeat watchdog still covers tty death if a registration fails.
fn watch_signal(signum: i32) -> Option<Arc<AtomicBool>> {
    let flag = Arc::new(AtomicBool::new(false));
    match flag::register(signum, Arc::clone(&flag)) {
        Ok(_) => Some(flag),
        Err(_) => None,
    }
}

/// Spawn the supervisor thread. On a trip it restores the terminal and exits the
/// process (the session is already persisted, so the user can `--continue`).
///
/// Polls every [`POLL_INTERVAL`]; on a healthy system the decision
/// ([`trip_reason`]) returns `None` forever. The thread is detached — it dies
/// with the process on a normal exit, and is the thing that *makes* the process
/// exit on an abnormal one.
pub(crate) fn spawn(heartbeat: Heartbeat, active: Arc<AtomicBool>) {
    let signals = [SIGHUP, SIGINT, SIGQUIT, SIGTERM]
        .iter()
        .filter_map(|&s| watch_signal(s))
        .collect::<Vec<_>>();
    let wedge_ms = WEDGE_TIMEOUT.as_millis() as u64;
    thread::spawn(move || loop {
        thread::sleep(POLL_INTERVAL);
        let any_signal = signals.iter().any(|f| f.load(Ordering::Relaxed));
        if let Some(reason) = trip_reason(
            heartbeat.now_ms(),
            heartbeat.last_ms(),
            any_signal,
            active.load(Ordering::Relaxed),
            wedge_ms,
        ) {
            // The tty is likely gone; stderr is usually not visible, but try.
            let _ = writeln!(
                std::io::stderr(),
                "opencode: input collector {reason:?} — restoring terminal and exiting. \
                 Reopen with `opencode --continue` to resume this session."
            );
            // Idempotent, tty-safe, swallows its own errors.
            TerminalGuard::restore();
            std::process::exit(0);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn trip_reason_no_trip_when_fresh_and_quiet() {
        let now = 10_000;
        let last = 9_990; // 10 ms stale — far under the 5 s wedge window
        assert_eq!(trip_reason(now, last, false, true, 5_000), None);
    }

    #[test]
    fn trip_reason_wedge_when_stale_and_active() {
        let now = 10_000;
        let last = 4_000; // 6 s stale > 5 s
        assert_eq!(
            trip_reason(now, last, false, true, 5_000),
            Some(Trip::InputWedge)
        );
    }

    #[test]
    fn trip_reason_ignores_staleness_during_shutdown() {
        // `active = false` (normal shutdown, pump dropped) → never a wedge.
        let now = 10_000;
        let last = 0; // very stale
        assert_eq!(trip_reason(now, last, false, false, 5_000), None);
    }

    #[test]
    fn trip_reason_signal_wins_regardless_of_active_or_staleness() {
        // Even mid-shutdown, a termination signal must trigger a clean exit.
        assert_eq!(trip_reason(0, 0, true, false, 5_000), Some(Trip::Signal));
        assert_eq!(trip_reason(9, 9, true, true, 5_000), Some(Trip::Signal));
    }

    #[test]
    fn trip_reason_boundary_is_strictly_greater() {
        // Exactly at the wedge window is NOT a trip (off-by-one safety).
        assert_eq!(trip_reason(5_000, 0, false, true, 5_000), None);
        assert_eq!(
            trip_reason(5_001, 0, false, true, 5_000),
            Some(Trip::InputWedge)
        );
    }
}

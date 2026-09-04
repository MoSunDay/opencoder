//! Terminal input collection — a dedicated OS thread running bounded
//! `crossterm::event::poll` + `read`, forwarding events over a tokio channel.
//!
//! This replaces `crossterm::event::EventStream`. The previous design polled
//! `EventStream::next()` directly inside the main `tokio::select!`. The async
//! stream's reader task (mio + tokio waker) could stall — once it stopped
//! resolving, the `select!` arm never fired, starving the whole event loop
//! (no keys, no Ctrl+C/D, process alive but wedged).
//!
//! The fix sidesteps the async layer entirely: a plain OS thread drives the
//! *synchronous* `event::poll(timeout)` + `event::read()`. This path is
//! bounded end to end — crossterm's unix source backs `poll` with
//! `filedescriptor::poll` + non-blocking reads, so there is no unbounded
//! `read()`; a lone `\x1b` commits as Esc immediately (`more=false`); and
//! `event::read()` after a successful `event::poll()` pops the already-queued
//! event without re-polling. The collector thread therefore wakes at least
//! every `POLL_TIMEOUT` and notices receiver-drop promptly. The wedge failure
//! mode is eliminated structurally — no watchdog, no stream rebuild.
//!
//! On top of that sits the **Esc-tail guard** ([`EscGuard`]): tmux/pty can
//! split an escape sequence across writes (ESC in one write, `[D` in the
//! next). crossterm 0.28 commits a lone `\x1b` as an Esc key immediately and
//! parses the orphan tail as plain characters, which the key handler then
//! typed into the input box as `[D` / `[A` garbage. The guard holds a lone
//! Esc for a short window to reassemble the split sequence before anything
//! reaches the UI.

use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::sync::mpsc;

/// Max time spent blocked in a single `poll`. Caps worst-case shutdown latency
/// and guarantees the collector re-evaluates `is_closed()` at least this often.
const POLL_TIMEOUT: Duration = Duration::from_millis(150);

/// Capacity of the event channel. Generous enough that a bursty paste never
/// drops input, small enough that a stalled main loop applies clear backpressure
/// (the collector blocks on `blocking_send` rather than losing keys).
const CHANNEL_CAPACITY: usize = 256;

/// How long a lone Esc is held while waiting for a disambiguating follow-up
/// before being committed as an Esc key. 80 ms is imperceptible for cursor
/// moves and input editing, and is in the same ballpark as neovim's
/// `ttimeoutlen` default (50 ms). A split CSI/SS3 tail virtually always
/// arrives within a single poll tick, so real sequences incur no visible
/// delay.
const ESC_GUARD_WINDOW: Duration = Duration::from_millis(80);

/// Esc-tail guard states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EscGuardState {
    /// No pending Esc.
    Idle,
    /// A bare Esc is held, waiting for the next event (or window expiry).
    Holding,
    /// Esc + `[`/`O` consumed: swallowing CSI/SS3 residue within the window.
    SwallowTail,
}

/// A bare Esc key press (`Event::Key` with `Esc`, plain modifiers).
fn esc_event() -> Event {
    Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
}

/// True for a real Esc key press (press kind, no modifiers).
fn is_esc(ev: &Event) -> bool {
    matches!(
        ev,
        Event::Key(KeyEvent {
            code: KeyCode::Esc,
            kind: KeyEventKind::Press,
            ..
        })
    )
}

/// True for the CSI (`ESC [`) / SS3 (`ESC O`) lead-in byte. When the pty
/// splits a sequence, the `[`/`O` arrives as its own key event right after
/// the held Esc.
fn is_csi_lead(ev: &Event) -> bool {
    matches!(
        ev,
        Event::Key(KeyEvent {
            code: KeyCode::Char('[') | KeyCode::Char('O'),
            kind: KeyEventKind::Press,
            ..
        })
    )
}

/// True for a CSI/SS3 tail byte as delivered by a split write: parameter and
/// final bytes (digits, letters, `;`, `~`, `<`, `>`, `?`). Eaten within the
/// swallow window so they never reach the UI as typed characters.
fn is_csi_residue(ev: &Event) -> bool {
    match ev {
        Event::Key(KeyEvent {
            code: KeyCode::Char(c),
            kind: KeyEventKind::Press,
            ..
        }) => c.is_ascii_alphanumeric() || matches!(c, ';' | '~' | '<' | '>' | '?'),
        _ => false,
    }
}

/// Pure Esc-tail state transition. `expired` is `true` exactly when the
/// caller measured the current window deadline as already passed before this
/// event arrived. Returns the (new state, events to emit, in order). Pure so
/// the whole reassembly logic is table-testable without a clock.
fn esc_guard_feed(state: EscGuardState, expired: bool, ev: &Event) -> (EscGuardState, Vec<Event>) {
    match state {
        EscGuardState::Idle => {
            if is_esc(ev) {
                // A lone Esc: hold it, emit nothing yet.
                (EscGuardState::Holding, Vec::new())
            } else {
                (EscGuardState::Idle, vec![ev.clone()])
            }
        }
        EscGuardState::Holding => {
            if expired {
                // Window elapsed: commit the held Esc, then treat the new
                // event as a fresh one (from Idle).
                let mut out = vec![esc_event()];
                let (s, rest) = esc_guard_feed(EscGuardState::Idle, false, ev);
                out.extend(rest);
                (s, out)
            } else if is_esc(ev) {
                // Esc + Esc within the window: two deliberate Escs — the
                // double-Esc cancel must reach the key handler intact.
                (EscGuardState::Idle, vec![esc_event(), esc_event()])
            } else if is_csi_lead(ev) {
                // Esc + [ or O: head of a CSI/SS3 sequence split by the pty.
                // Consume both; swallow the tail next.
                (EscGuardState::SwallowTail, Vec::new())
            } else {
                // Some other key: a real Esc followed by a real key.
                (EscGuardState::Idle, vec![esc_event(), ev.clone()])
            }
        }
        EscGuardState::SwallowTail => {
            if is_esc(ev) {
                // A new Esc during tail swallow: hold it for its own
                // disambiguation (preserves double-Esc cancel).
                (EscGuardState::Holding, Vec::new())
            } else if expired || !is_csi_residue(ev) {
                // Window over, or a non-CSI byte: the residue phase is done,
                // the event is a real key.
                (EscGuardState::Idle, vec![ev.clone()])
            } else {
                // CSI/SS3 residue char: eat it.
                (EscGuardState::SwallowTail, Vec::new())
            }
        }
    }
}

/// Runtime driver for the Esc guard: tracks the window deadline and applies
/// the pure [`esc_guard_feed`] transition to live events. The pump uses
/// [`EscGuard::poll_timeout`] so a held Esc is committed at the deadline even
/// when no further input arrives.
#[derive(Debug, Clone, Copy)]
struct EscGuard {
    state: EscGuardState,
    deadline: Option<Instant>,
}

impl EscGuard {
    fn new() -> Self {
        Self {
            state: EscGuardState::Idle,
            deadline: None,
        }
    }

    /// Poll timeout honouring the window: while holding/swallowing, wake at
    /// the deadline (not just at [`POLL_TIMEOUT`]) so the held Esc is
    /// committed without extra latency beyond [`ESC_GUARD_WINDOW`].
    fn poll_timeout(&self, base: Duration) -> Duration {
        match self.deadline {
            Some(d) => d.saturating_duration_since(Instant::now()).min(base),
            None => base,
        }
    }

    /// Commit any held Esc whose window expired — called whenever we wake up
    /// (poll timeout or an event) and find the deadline passed while blocked.
    fn flush_expired(&mut self) -> Option<Event> {
        if !self.expired() {
            return None;
        }
        match self.state {
            EscGuardState::Holding => {
                // Window elapsed: commit the held Esc.
                self.state = EscGuardState::Idle;
                self.deadline = None;
                Some(esc_event())
            }
            EscGuardState::SwallowTail => {
                // The held Esc was consumed reassembling a CSI tail that never
                // arrived within the window. Nothing to commit — just return
                // to Idle and clear the deadline so poll_timeout stops
                // returning Duration::ZERO (which would busy-loop the pump).
                self.state = EscGuardState::Idle;
                self.deadline = None;
                None
            }
            EscGuardState::Idle => None,
        }
    }

    /// Feed one raw event; returns the events the pump must forward.
    fn feed(&mut self, ev: Event) -> Vec<Event> {
        let expired = self.expired();
        let (state, out) = esc_guard_feed(self.state, expired, &ev);
        self.state = state;
        self.deadline = match state {
            EscGuardState::Holding | EscGuardState::SwallowTail => {
                Some(Instant::now() + ESC_GUARD_WINDOW)
            }
            EscGuardState::Idle => None,
        };
        out
    }

    fn expired(&self) -> bool {
        match self.deadline {
            Some(d) => Instant::now() >= d,
            None => false,
        }
    }
}

/// Spawn the input collector thread.
///
/// Returns the receiving end (to be polled in the main `select!`) and the
/// thread handle. The thread exits on its own when the receiver is dropped
/// (detected via `Sender::is_closed()` on every poll cycle) or when stdin
/// reports a read error. Drop the receiver to shut it down.
pub fn spawn_input_pump(
    heartbeat: crate::supervisor::Heartbeat,
) -> (mpsc::Receiver<Event>, thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel::<Event>(CHANNEL_CAPACITY);
    let handle = thread::spawn(move || {
        // Esc-tail guard: tmux/pty may split an escape sequence across writes
        // (a bare \x1b commits as Esc immediately in crossterm 0.28; the `[D`
        // tail arrives as separate key events). The guard holds a lone Esc for
        // a short window to reassemble the sequence before it hits the UI.
        let mut esc_guard = EscGuard::new();
        loop {
            // Bumped *before* the blocking poll: if the poll never returns
            // (crossterm 0.28's mio source busy-loops forever on tty EOF/EIO,
            // holding the global event mutex), the bump stops and the liveness
            // supervisor restores the terminal + exits instead of freezing.
            heartbeat.bump();
            // Receiver gone? Shut down without touching the terminal. Checked
            // every iteration so an idle stream (no events) still exits promptly.
            if tx.is_closed() {
                break;
            }
            // Bounded poll: returns within the timeout regardless of whether
            // an event arrived (crossterm backs this with
            // `filedescriptor::poll` + non-blocking reads). While the guard
            // holds an Esc the timeout shrinks to the remaining window, so the
            // held Esc is committed at the deadline even without input.
            // `false` → loop and re-check `is_closed()`; `Err` (e.g. stdin
            // closed / TTY lost) → break to avoid a busy-spin.
            let ready = match event::poll(esc_guard.poll_timeout(POLL_TIMEOUT)) {
                Ok(v) => v,
                // Silently exit on poll failure (e.g. TTY lost): writing to
                // stderr while the alt screen is active corrupts the display.
                // Mirrors the silent `break` already used for `event::read()`
                // errors below.
                Err(_) => break,
            };
            // A window may have elapsed while blocked inside poll(): commit
            // the held Esc now, before processing whatever made us wake.
            if let Some(ev) = esc_guard.flush_expired() {
                if tx.blocking_send(ev).is_err() {
                    break;
                }
            }
            if !ready {
                continue;
            }
            // `read()` is safe here (not unbounded): the successful `poll()`
            // above already queued a complete event, so `read()` pops it from
            // the internal queue immediately — it never reaches its own
            // `poll(None)` fallback path.
            match event::read() {
                Ok(ev) => {
                    // blocking_send is legal on a dedicated OS thread (not
                    // inside a runtime worker). Err ⇒ receiver dropped
                    // mid-send ⇒ exit.
                    for out in esc_guard.feed(ev) {
                        if tx.blocking_send(out).is_err() {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });
    (rx, handle)
}

/// How long the shutdown drain waits without an event before declaring the
/// input stream quiet. Key-release reports arrive within tens of milliseconds
/// of the quitting keypress, so 80 ms absorbs them without delaying exit.
const DRAIN_QUIET: Duration = Duration::from_millis(80);

/// Hard cap on the shutdown drain. Bounds the added exit latency even when
/// input keeps arriving (held key auto-repeat, a drag) so quitting never
/// feels wedged.
const DRAIN_CAP: Duration = Duration::from_millis(300);

/// Quit-path input absorb: quiesce the terminal's reporting modes, then drain
/// the event channel for a bounded quiet window, discarding everything still
/// in flight.
///
/// Why: the quitting keypress (Ctrl+D, the final Enter of `/exit`) is still
/// physically held when shutdown starts. Under the Kitty keyboard protocol
/// (pushed by [`crate::terminal::TerminalGuard::enter`]) its release/repeat
/// arrives as `CSI ..;1:3u` reports; events that land after the pump stops are
/// stranded in the tty input queue and — outside tmux, which discards the
/// pane's leftover input — the shell echoes the CSI tails as garbage
/// (`442;1:3u`, `0;1:3u`) at the prompt. The quiesce pop stops new reports
/// from being generated; this drain absorbs the ones already in flight. Events
/// typed ahead during the (≤ [`DRAIN_CAP`]) window are discarded too — the app
/// is quitting, so there is nothing left to deliver them to.
///
/// The channel (not the tty) is drained so the pump thread stays the only
/// crossterm reader; `rx` must not have another consumer at this point (the
/// main loop has already exited).
pub(crate) async fn drain_shutdown(rx: &mut mpsc::Receiver<Event>) {
    crate::terminal::quiesce_input_reporting();
    drain_until_quiet(rx, DRAIN_QUIET, DRAIN_CAP).await;
}

/// Bounded quiet-window drain over the event channel. Discards every event;
/// stops at the first quiet window (no event for `quiet`), at the `cap`
/// deadline, or when the channel closes — whichever comes first. Isolated from
/// [`drain_shutdown`] (no tty side effects) so tests can shrink the durations
/// and stay deterministic.
async fn drain_until_quiet(rx: &mut mpsc::Receiver<Event>, quiet: Duration, cap: Duration) {
    let deadline = Instant::now() + cap;
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let wait = quiet.min(deadline - now);
        match tokio::time::timeout(wait, rx.recv()).await {
            // Event in flight: discard it and restart the quiet window.
            Ok(Some(_)) => continue,
            // Channel closed (pump gone): nothing left to absorb.
            Ok(None) => break,
            // Quiet window elapsed with nothing arriving: input stream is dry.
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dropping the receiver must release the collector thread promptly (within
    /// a couple of poll windows). This is the shutdown contract the main loop
    /// relies on: ending `run_app` drops the receiver, the thread exits, no leak.
    #[test]
    fn pump_exits_when_receiver_dropped() {
        let (rx, handle) = spawn_input_pump(crate::supervisor::Heartbeat::new());
        drop(rx);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if handle.is_finished() {
                return;
            }
            if Instant::now() > deadline {
                panic!("input pump did not shut down after receiver drop");
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    // ── Shutdown drain (bounded quiet-window absorb) ──

    async fn push_all(tx: &tokio::sync::mpsc::Sender<Event>, evs: &[Event]) {
        for ev in evs {
            tx.send(ev.clone()).await.unwrap();
        }
    }

    /// Queued in-flight events (kitty release/repeat reports of the quitting
    /// keypress, mouse, paste) are all discarded; after the quiet window the
    /// channel is empty.
    #[tokio::test]
    async fn drain_discards_in_flight_events_then_stops_on_quiet() {
        let (tx, mut rx) = mpsc::channel::<Event>(16);
        push_all(
            &tx,
            &[
                key(KeyCode::Enter),
                key(KeyCode::Esc),
                key(KeyCode::Char('d')),
            ],
        )
        .await;
        // Keep the sender alive: dropping it would hit the (instant)
        // channel-closed branch — covered by
        // `drain_returns_promptly_when_channel_closed` — and skip the
        // quiet-window wait this test exists to verify.
        let _keep_alive = &tx;

        let started = Instant::now();
        drain_until_quiet(&mut rx, Duration::from_millis(20), Duration::from_secs(5)).await;

        assert!(
            started.elapsed() >= Duration::from_millis(20),
            "must wait out the quiet window before declaring the stream dry"
        );
        assert!(
            rx.try_recv().is_err(),
            "every in-flight event must be discarded, none left for the shell"
        );
    }

    /// Input arriving continuously (held-key auto-repeat) must not stretch the
    /// drain past the hard cap — quitting stays bounded even under fire.
    #[tokio::test]
    async fn drain_is_bounded_by_cap_when_events_keep_arriving() {
        let (tx, mut rx) = mpsc::channel::<Event>(256);
        let feeder = tokio::spawn(async move {
            for _ in 0..100 {
                if tx.send(key(KeyCode::Char('x'))).await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });

        let started = Instant::now();
        drain_until_quiet(
            &mut rx,
            Duration::from_millis(80),
            Duration::from_millis(150),
        )
        .await;
        let elapsed = started.elapsed();

        assert!(
            elapsed >= Duration::from_millis(140),
            "cap window must actually elapse under continuous input: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(600),
            "drain must stay bounded by the cap, not by the feeder: {elapsed:?}"
        );
        feeder.abort();
    }

    /// A closed channel (pump already gone) must end the drain immediately —
    /// no point waiting out the quiet window on nothing.
    #[tokio::test]
    async fn drain_returns_promptly_when_channel_closed() {
        let (tx, mut rx) = mpsc::channel::<Event>(4);
        drop(tx);

        let started = Instant::now();
        drain_until_quiet(&mut rx, Duration::from_secs(5), Duration::from_secs(10)).await;
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "closed channel must not wait out the quiet window"
        );
    }

    // ── Esc-tail guard (pure state machine) ──

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }
    fn esc() -> Event {
        key(KeyCode::Esc)
    }
    fn ch(c: char) -> Event {
        key(KeyCode::Char(c))
    }

    /// Feed the pure transition step-by-step; each step's bool says whether
    /// the window had expired before that event arrived.
    fn run(steps: &[(bool, Event)]) -> Vec<Event> {
        let mut state = EscGuardState::Idle;
        let mut out = Vec::new();
        for (expired, ev) in steps {
            let (s, mut o) = esc_guard_feed(state, *expired, ev);
            state = s;
            out.append(&mut o);
        }
        out
    }

    #[test]
    fn esc_guard_single_esc_held_then_committed_on_expiry() {
        // A lone Esc emits nothing while held; expiry (no follow-up event)
        // commits it — that is the pump's flush_expired path.
        let out = run(&[(false, esc())]);
        assert_eq!(out, Vec::<Event>::new());
        // An event arriving after expiry flushes the held Esc first.
        let out = run(&[(false, esc()), (true, ch('x'))]);
        assert_eq!(out, vec![esc(), ch('x')]);
    }

    #[test]
    fn esc_guard_double_esc_passes_both() {
        // Esc + Esc within the window: the double-Esc cancel must reach the
        // key handler intact.
        let out = run(&[(false, esc()), (false, esc())]);
        assert_eq!(out, vec![esc(), esc()]);
    }

    #[test]
    fn esc_guard_swallows_split_csi_arrow() {
        // tmux/pty split: Esc, then `[`, then `D` in separate writes. Without
        // the guard this would type `[D` into the input box.
        let out = run(&[(false, esc()), (false, ch('[')), (false, ch('D'))]);
        assert_eq!(out, Vec::<Event>::new());
    }

    #[test]
    fn esc_guard_swallows_split_ss3() {
        // Application cursor keys use ESC O D — same split pattern.
        let out = run(&[(false, esc()), (false, ch('O')), (false, ch('D'))]);
        assert_eq!(out, Vec::<Event>::new());
    }

    #[test]
    fn esc_guard_swallows_split_insert_key() {
        // ESC [ 2 ~ (Insert), split across writes.
        let out = run(&[
            (false, esc()),
            (false, ch('[')),
            (false, ch('2')),
            (false, ch('~')),
        ]);
        assert_eq!(out, Vec::<Event>::new());
    }

    #[test]
    fn esc_guard_esc_then_normal_key_passes_both() {
        // Esc followed quickly by a real key (e.g. the user pressed Esc then
        // typed 'a'): both are forwarded.
        let out = run(&[(false, esc()), (false, ch('a'))]);
        assert_eq!(out, vec![esc(), ch('a')]);
    }

    #[test]
    fn esc_guard_residue_stops_at_window_expiry() {
        // Residue is only swallowed within the window; a byte arriving after
        // expiry is a real key.
        let out = run(&[(false, esc()), (false, ch('[')), (true, ch('D'))]);
        assert_eq!(out, vec![ch('D')]);
    }

    #[test]
    fn esc_guard_non_key_event_after_esc_flushes() {
        // Paste/mouse events are never CSI residue: the held Esc is flushed
        // first, then the event passes through.
        let out = run(&[(false, esc()), (false, Event::Paste("x".into()))]);
        assert_eq!(out, vec![esc(), Event::Paste("x".into())]);
    }

    // ── flush_expired (deadline-boundary transitions on the live EscGuard) ──

    #[test]
    fn flush_expired_clears_swallow_tail_when_window_passes() {
        // Regression for the 100% CPU busy-loop: a guard stuck in SwallowTail
        // (held Esc consumed while reassembling a CSI tail) whose window
        // passes must return to Idle and clear its deadline. Otherwise
        // poll_timeout keeps returning Duration::ZERO and the input pump
        // spins forever.
        let mut g = EscGuard::new();
        g.state = EscGuardState::SwallowTail;
        g.deadline = Some(Instant::now() - Duration::from_millis(1));
        // An expired SwallowTail yields no event (the Esc was consumed) ...
        assert_eq!(g.flush_expired(), None);
        // ... but it MUST transition to Idle and clear the deadline.
        assert_eq!(g.state, EscGuardState::Idle);
        assert_eq!(g.deadline, None);
    }

    #[test]
    fn flush_expired_commits_holding_when_expired() {
        // Regression guard for the pre-existing behaviour: an expired Holding
        // guard still commits the held Esc as a real key event.
        let mut g = EscGuard::new();
        g.state = EscGuardState::Holding;
        g.deadline = Some(Instant::now() - Duration::from_millis(1));
        assert_eq!(g.flush_expired(), Some(esc_event()));
        assert_eq!(g.state, EscGuardState::Idle);
        assert_eq!(g.deadline, None);
    }

    #[test]
    fn flush_expired_returns_none_when_not_expired() {
        // A guard whose window has not yet passed yields nothing and leaves
        // its state/deadline untouched.
        let mut g = EscGuard::new();
        g.state = EscGuardState::Holding;
        let dl = Instant::now() + Duration::from_secs(1);
        g.deadline = Some(dl);
        assert_eq!(g.flush_expired(), None);
        assert_eq!(g.state, EscGuardState::Holding);
        assert_eq!(g.deadline, Some(dl));

        // Likewise for a SwallowTail guard still within the window.
        g.state = EscGuardState::SwallowTail;
        assert_eq!(g.flush_expired(), None);
        assert_eq!(g.state, EscGuardState::SwallowTail);
    }
}

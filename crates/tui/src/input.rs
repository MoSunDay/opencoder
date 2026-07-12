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

use std::thread;
use std::time::Duration;

use crossterm::event::{self, Event};
use tokio::sync::mpsc;

/// Max time spent blocked in a single `poll`. Caps worst-case shutdown latency
/// and guarantees the collector re-evaluates `is_closed()` at least this often.
const POLL_TIMEOUT: Duration = Duration::from_millis(150);

/// Capacity of the event channel. Generous enough that a bursty paste never
/// drops input, small enough that a stalled main loop applies clear backpressure
/// (the collector blocks on `blocking_send` rather than losing keys).
const CHANNEL_CAPACITY: usize = 256;

/// Spawn the input collector thread.
///
/// Returns the receiving end (to be polled in the main `select!`) and the
/// thread handle. The thread exits on its own when the receiver is dropped
/// (detected via `Sender::is_closed()` on every poll cycle) or when stdin
/// reports a read error. Drop the receiver to shut it down.
pub fn spawn_input_pump() -> (mpsc::Receiver<Event>, thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel::<Event>(CHANNEL_CAPACITY);
    let handle = thread::spawn(move || loop {
        // Receiver gone? Shut down without touching the terminal. Checked every
        // iteration so an idle stream (no events) still exits promptly.
        if tx.is_closed() {
            break;
        }
        // Bounded poll: returns within POLL_TIMEOUT regardless of whether an
        // event arrived (crossterm backs this with `filedescriptor::poll` +
        // non-blocking reads). `false`/err → loop and re-check `is_closed()`.
        if !event::poll(POLL_TIMEOUT).unwrap_or(false) {
            continue;
        }
        // `read()` is safe here (not unbounded): the successful `poll()` above
        // already queued a complete event, so `read()` pops it from the
        // internal queue immediately — it never reaches its own `poll(None)`
        // fallback path.
        match event::read() {
            Ok(ev) => {
                // blocking_send is legal on a dedicated OS thread (not inside a
                // runtime worker). Err ⇒ receiver dropped mid-send ⇒ exit.
                if tx.blocking_send(ev).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    });
    (rx, handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Dropping the receiver must release the collector thread promptly (within
    /// a couple of poll windows). This is the shutdown contract the main loop
    /// relies on: ending `run_app` drops the receiver, the thread exits, no leak.
    #[test]
    fn pump_exits_when_receiver_dropped() {
        let (rx, handle) = spawn_input_pump();
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
}

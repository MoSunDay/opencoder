//! Characterization test for the input collector's basic delivery contract.
//!
//! Pins down that a lone `\x1b` (Esc) is surfaced as an event within a bounded
//! time on the crossterm 0.28 **synchronous** `poll`/`read` path — the path the
//! freeze fix switched to. This is a regression guard that the collector thread
//! end-to-end forwards input promptly on a pty.
//!
//! What this test does NOT do: it does not reproduce the original freeze. The
//! freeze was a stall in the `EventStream` async/mio layer, not a sync-path
//! parser wedge (a lone `\x1b` with `more=false` commits as Esc immediately —
//! see crossterm `parse.rs:36-42`). The structural "pump survives partial
//! input" half of the invariant is covered by the companion test
//! `input_pty_incomplete.rs`. The two together bound the fix's core claim.

#![cfg(unix)]

mod common;

use std::time::Duration;

use crossterm::event::{Event, KeyCode};

use common::PtyStdin;

/// Inject a lone Esc over a pty wired to stdin and assert the collector
/// surfaces it within 2s. A hang here is exactly the regression this guards.
#[tokio::test]
async fn lone_esc_is_delivered_within_bound() {
    let pty = PtyStdin::open();

    let (mut rx, _handle) = opencode_tui::input::spawn_input_pump();
    // NOTE: `_handle` is intentionally detached (not joined). If the collector
    // ever wedged (the regression this family of tests guards), joining would
    // hang the test forever; detaching lets the recv-timeout fail fast and the
    // process exit reaps the thread.

    pty.inject(b"\x1b");

    // Bound the wait; a wedge would let this time out.
    let got = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await;

    // Drop the receiver so the collector exits; the pty is restored on drop.
    // Asserting after cleanup so a failed assert never leaks the fd redirect.
    drop(rx);

    match got {
        Ok(Some(Event::Key(k))) => assert_eq!(
            k.code,
            KeyCode::Esc,
            "expected lone \\x1b to parse as Esc, got {k:?}"
        ),
        other => panic!("lone Esc was NOT delivered within 2s — collector wedged: {other:?}"),
    }
}

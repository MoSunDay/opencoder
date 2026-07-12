//! Characterization test: the input collector must NOT wedge on an incomplete
//! escape sequence.
//!
//! This is the structural half of the freeze fix's invariant. crossterm's
//! parser, on seeing `\x1b[` (a partial CSI), buffers it and returns
//! `Ok(None)` — "wait for more bytes" (confirmed: crossterm 0.28
//! `parse.rs:140-141`). The collector's bounded `poll(150ms)` then times out
//! with no complete event and the thread cycles back to re-check `is_closed()`.
//! It must NOT get stuck.
//!
//! The test injects `\x1b[`, waits well beyond the poll window (so the pump has
//! definitely timed out and cycled at least once on the partial sequence),
//! then injects the completing byte `A`. crossterm combines it with the
//! buffered `\x1b[` into an Up-arrow event (`\x1b[A`). If the pump had wedged
//! on the incomplete sequence, the completing byte would never be read and the
//! 2s recv-timeout would fail the test.

#![cfg(unix)]

mod common;

use std::time::Duration;

use crossterm::event::{Event, KeyCode};

use common::PtyStdin;

/// An incomplete CSI must not wedge the collector; the completing byte is
/// delivered once it arrives.
#[tokio::test]
async fn incomplete_csi_does_not_wedge_pump() {
    let pty = PtyStdin::open();

    let (mut rx, _handle) = opencoder_tui::input::spawn_input_pump();
    // `_handle` detached — see `input_pty.rs` for the rationale.

    // Partial CSI: parser buffers `\x1b[`, returns Ok(None). The pump's
    // poll(150ms) times out — no complete event — and the thread cycles.
    pty.inject(b"\x1b[");

    // Sleep well beyond the 150ms poll window so the pump has definitely
    // timed out on the incomplete sequence and cycled. If the thread had
    // wedged inside poll/read here, this delay would not help and the later
    // recv would time out — failing the test.
    tokio::time::sleep(Duration::from_millis(400)).await;

    // Inject the completing byte: buffered `\x1b[` + `A` => `\x1b[A` = Up.
    pty.inject(b"A");

    let got = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await;
    drop(rx);

    match got {
        Ok(Some(Event::Key(k))) => assert_eq!(
            k.code,
            KeyCode::Up,
            "expected Up (from completed CSI \\x1b[A), got {k:?}"
        ),
        other => {
            panic!("pump wedged on incomplete CSI — no event after completing byte: {other:?}")
        }
    }
}

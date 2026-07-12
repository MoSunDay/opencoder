//! Characterization test for the input collector — empirically pins down the
//! core invariant the freeze fix relies on: a lone `\x1b` (Esc) must be
//! delivered as an event within a bounded time on the real crossterm 0.28 in
//! use, i.e. `poll`+`read` does NOT wedge on the Kitty disambiguation path.
//!
//! Lives in its own integration-test binary so it owns a private process: the
//! test redirects fd 0 (STDIN_FILENO) onto a pty slave so crossterm's
//! `tty_fd()` — which uses fd 0 when `isatty(0)` is true (confirmed in
//! crossterm 0.28.1 `file_descriptor.rs:124-138`) — reads our injected bytes.
//! That fd-0 mutation is process-global and must not touch other tests.
//!
//! If a crossterm upgrade ever makes `poll` ignore its timeout under Esc
//! disambiguation, this test fails instead of the user discovering a frozen
//! TUI.

#![cfg(unix)]

use std::time::Duration;

use crossterm::event::{Event, KeyCode};

/// Inject a lone Esc over a pty wired to stdin and assert the collector
/// surfaces it within 2s. A hang here is exactly the regression this guards.
#[tokio::test]
async fn lone_esc_is_delivered_within_bound() {
    // --- open a pty pair (master/slave) ---
    let mut master: libc::c_int = -1;
    let mut slave: libc::c_int = -1;
    let r = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    assert_eq!(r, 0, "openpty failed (crossterm pty harness unavailable)");

    // --- put the slave into raw mode so `\x1b` is readable without canonical
    //     line buffering (a lone Esc would otherwise sit until a newline) ---
    let mut orig_termios: libc::termios = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe { libc::tcgetattr(slave, &mut orig_termios) },
        0,
        "tcgetattr failed"
    );
    let mut raw = orig_termios;
    unsafe { libc::cfmakeraw(&mut raw) };
    assert_eq!(
        unsafe { libc::tcsetattr(slave, libc::TCSANOW, &raw) },
        0,
        "tcsetattr(raw) failed"
    );

    // --- save real stdin, redirect fd 0 onto the pty slave so crossterm reads
    //     our pty (its tty_fd() picks fd 0 when isatty(0) is true) ---
    let saved_stdin = unsafe { libc::dup(libc::STDIN_FILENO) };
    assert!(saved_stdin >= 0, "dup(stdin) failed");
    let dup2_rc = unsafe { libc::dup2(slave, libc::STDIN_FILENO) };
    assert_eq!(
        dup2_rc, 0,
        "dup2(slave -> stdin) failed: {dup2_rc}"
    );

    // --- start the collector; it now reads from our pty via fd 0 ---
    let (mut rx, _handle) = opencode_tui::input::spawn_input_pump();
    // NOTE: `_handle` is intentionally detached (not joined). If the collector
    // ever wedges (the very regression this test guards), joining would hang
    // the test forever; detaching lets the 2s recv-timeout fail the test fast
    // and the process exit reaps the thread.

    // --- inject a lone Esc on the master end ---
    let written = unsafe { libc::write(master, b"\x1b".as_ptr() as *const _, 1) };
    assert_eq!(written, 1, "write(\\x1b) to pty master failed");

    // --- bound the wait; a wedge would let this time out ---
    let got = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await;

    // --- unconditional cleanup: restore fd 0 + termios, close pty fds. The
    //     collector is detached; dropping `rx` asks it to exit, but we do NOT
    //     join (see note above) so a wedged collector can't hang the test. ---
    unsafe {
        libc::dup2(saved_stdin, libc::STDIN_FILENO);
        libc::close(saved_stdin);
        libc::tcsetattr(slave, libc::TCSANOW, &orig_termios);
        libc::close(master);
        libc::close(slave);
    }
    drop(rx);

    // --- assert after cleanup so a failed assert never leaks the fd redirect ---
    match got {
        Ok(Some(Event::Key(k))) => assert_eq!(
            k.code,
            KeyCode::Esc,
            "expected lone \\x1b to parse as Esc, got {k:?}"
        ),
        other => panic!("lone Esc was NOT delivered within 2s — collector wedged: {other:?}"),
    }
}

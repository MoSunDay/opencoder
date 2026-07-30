//! Isolated coverage for `bg::kill_all`.
//!
//! This lives in its OWN integration-test binary (a separate process-global
//! registry) and contains a single test, so the global-draining `kill_all`
//! cannot race with another test's registered command. The unit tests in
//! `tools/bg.rs` exercise per-pid `stop`/`unregister` only; `kill_all` is the
//! one genuinely global operation and needs this isolation to stay robust under
//! parallel test execution.

use std::process::Command;
use std::time::Duration;

use opencoder_session::tools::bg::{kill_all, list, register};

/// `kill_all` signals every registered process group and drains the registry:
/// after it returns, `list()` no longer contains the pids it killed.
#[cfg(unix)]
#[test]
fn kill_all_drains_and_signals_registered_group() {
    // `setsid` makes pgid == pid so the group kill is scoped to this child only.
    let mut child = Command::new("setsid")
        .args(["sleep", "60"])
        .spawn()
        .expect("spawn setsid sleep");
    let pid = child.id();
    let pgid = pid as libc::pid_t;
    // Give setsid()+exec a moment to establish the new session.
    std::thread::sleep(Duration::from_millis(50));

    register(pid, pgid, "isolated".to_string());
    assert!(
        list().iter().any(|info| info.pid == pid),
        "list should expose the registered pid"
    );

    let killed = kill_all();
    assert!(
        killed >= 1,
        "kill_all should report at least the one group we registered"
    );
    assert!(
        !list().iter().any(|info| info.pid == pid),
        "kill_all should have drained the registry"
    );

    // Reap the killed child so we don't leave a zombie.
    let _ = child.wait();
}

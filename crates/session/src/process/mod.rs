//! Linux process ownership for node-launched tools.
//!
//! A configured node starts every external workload below the agent binary's
//! hidden supervisor. The supervisor is a child subreaper and holds a lease
//! pipe whose write end exists only in the node. Kernel-close on `SIGKILL`
//! therefore gives the supervisor a reliable crash notification.
//!
//! Real supervision depends on Linux-only kernel interfaces (pidfd,
//! `PR_SET_CHILD_SUBREAPER`, `PR_SET_PDEATHSIG`). Non-Linux platforms get the
//! fail-closed `fallback` stub: constructors keep returning their
//! unsupervised result and callers continue to run direct child processes.

#[cfg(target_os = "linux")]
mod owned;
#[cfg(target_os = "linux")]
mod pidfd;
#[cfg(target_os = "linux")]
mod supervisor;
#[cfg(target_os = "linux")]
mod tracker;

#[cfg(not(target_os = "linux"))]
mod fallback;

#[cfg(target_os = "linux")]
pub use owned::{command, configure_supervisor_binary, runc_command, OwnedSupervisor, SpawnLease};
#[cfg(target_os = "linux")]
pub use pidfd::SignalTarget;
#[cfg(target_os = "linux")]
pub use supervisor::{supervisor_main, RuncCleanup};
#[cfg(target_os = "linux")]
pub use tracker::{active_owned_processes, wait_for_owned_processes};

#[cfg(not(target_os = "linux"))]
pub use fallback::{
    active_owned_processes, command, configure_supervisor_binary, runc_command, supervisor_main,
    wait_for_owned_processes, OwnedSupervisor, RuncCleanup, SignalTarget, SpawnLease,
};

#[cfg(target_os = "linux")]
use anyhow::{Context, Result};
#[cfg(target_os = "linux")]
use std::ffi::{OsStr, OsString};
use tokio::process::Command;

/// Keep a hidden supervisor alive on outer future/handle drop so it can reap
/// descendants. Direct child fallbacks retain Tokio's kill-on-drop behavior.
pub fn configure_owned_command(command: &mut Command, supervised: bool) {
    command.kill_on_drop(!supervised);
}

/// Wait for a hidden supervisor to finish its cleanup. Only direct child
/// fallbacks may be force-killed by the outer runtime.
pub async fn wait_owned_child(
    child: &mut tokio::process::Child,
    supervised: bool,
) -> std::io::Result<std::process::ExitStatus> {
    if !supervised {
        child.start_kill()?;
    }
    child.wait().await
}

// Only the Linux supervisor consumes this split; keep it out of non-Linux
// builds so the fail-closed facade stays dead-code clean.
#[cfg(target_os = "linux")]
pub(crate) fn supervisor_args(command: &[OsString]) -> Result<(&OsStr, &[OsString])> {
    let (program, args) = command
        .split_first()
        .context("supervised command missing")?;
    Ok((program.as_os_str(), args))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::process::Stdio;

    #[tokio::test]
    async fn outer_timeout_never_force_kills_a_supervised_owner() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "while :; do sleep 1; done"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_owned_command(&mut command, true);
        let mut child = command.spawn().unwrap();
        let pid = child.id().unwrap();
        let identity = pidfd::start_time(pid).unwrap();
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(25),
            wait_owned_child(&mut child, true)
        )
        .await
        .is_err());
        assert_eq!(pidfd::start_time(pid), Some(identity));
        child.start_kill().unwrap();
        child.wait().await.unwrap();
    }
}

//! Non-Linux fail-closed stub: these platforms have no pidfd or child
//! subreaper, so node process supervision cannot be offered. Constructors
//! return `Ok(None)` (callers run a direct child process) or an explicit
//! error, matching the Linux path where no supervisor was configured.

use anyhow::{bail, Result};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};
use tokio::process::Command;

#[derive(Clone, Debug)]
pub struct RuncCleanup {
    pub root: PathBuf,
    pub id: String,
}

pub struct SpawnLease {
    _private: (),
}

impl SpawnLease {
    pub fn spawned(self, _pid: Option<u32>) -> Result<OwnedSupervisor> {
        bail!("process supervision is Linux-only")
    }
}

pub struct OwnedSupervisor {
    _private: (),
}

impl OwnedSupervisor {
    pub fn signal_target(&self) -> Result<SignalTarget> {
        bail!("process supervision is Linux-only")
    }

    pub fn terminate(&mut self) {}
}

pub struct SignalTarget {
    _private: (),
}

impl SignalTarget {
    pub fn signal(&self, _signal: libc::c_int) -> Result<()> {
        bail!("process supervision is Linux-only")
    }
}

pub fn configure_supervisor_binary(path: PathBuf) -> Result<()> {
    bail!(
        "process supervisor {} rejected: process supervision is Linux-only",
        path.display()
    )
}

/// Supervision is unavailable, so callers retain the direct-process behavior.
pub fn command(_program: impl AsRef<OsStr>) -> Result<Option<(Command, SpawnLease)>> {
    Ok(None)
}

/// Supervision is unavailable, so callers retain the direct-process behavior.
pub fn runc_command(
    _program: impl AsRef<OsStr>,
    _root: &Path,
    _id: &str,
) -> Result<Option<(Command, SpawnLease)>> {
    Ok(None)
}

pub fn supervisor_main(_command: Vec<OsString>, _cleanup: Option<RuncCleanup>) -> Result<i32> {
    bail!("process supervision is Linux-only")
}

pub fn active_owned_processes() -> usize {
    0
}

pub async fn wait_for_owned_processes(_deadline: tokio::time::Instant) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_report_no_supervised_command() {
        assert!(command("bash").unwrap().is_none());
        assert!(runc_command("runc", Path::new("/tmp"), "id")
            .unwrap()
            .is_none());
    }

    #[test]
    fn configuration_and_entrypoint_fail_closed() {
        assert!(configure_supervisor_binary(PathBuf::from("/bin/false")).is_err());
        assert!(supervisor_main(vec![OsString::from("sh")], None).is_err());
    }

    #[test]
    fn tracker_reports_no_owned_processes() {
        assert_eq!(active_owned_processes(), 0);
    }

    #[tokio::test]
    async fn owned_process_drain_completes_immediately() {
        wait_for_owned_processes(tokio::time::Instant::now())
            .await
            .unwrap();
    }

    #[test]
    fn lease_and_supervisor_api_fail_closed() {
        assert!(SpawnLease { _private: () }.spawned(Some(42)).is_err());
        let supervisor = OwnedSupervisor { _private: () };
        assert!(supervisor.signal_target().is_err());
        let mut supervisor = supervisor;
        supervisor.terminate();
        supervisor.terminate();
        assert!(SignalTarget { _private: () }.signal(libc::SIGTERM).is_err());
    }
}

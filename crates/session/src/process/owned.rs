//! Linux backend: build the commands that run below the hidden supervisor.

use super::{pidfd::SignalTarget, supervisor::RuncCleanup, tracker};
use anyhow::{Context, Result};
use std::{
    ffi::OsStr,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::{Path, PathBuf},
    sync::OnceLock,
};
use tokio::process::Command;

static SUPERVISOR_BINARY: OnceLock<PathBuf> = OnceLock::new();
pub(super) const LEASE_FD: libc::c_int = 3;

/// Configure the hidden supervisor used by node workloads. Calling this with
/// a different binary is rejected so tests and embedders cannot silently
/// replace an already-established process ownership boundary.
pub fn configure_supervisor_binary(path: PathBuf) -> Result<()> {
    let path = std::fs::canonicalize(&path)
        .with_context(|| format!("resolve process supervisor {}", path.display()))?;
    if let Some(existing) = SUPERVISOR_BINARY.get() {
        anyhow::ensure!(existing == &path, "process supervisor already configured");
        return Ok(());
    }
    SUPERVISOR_BINARY
        .set(path)
        .map_err(|_| anyhow::anyhow!("process supervisor configuration raced"))
}

/// Return a supervised command when the node configured its agent binary.
/// Other library embedders retain the existing direct-process behavior.
pub fn command(program: impl AsRef<OsStr>) -> Result<Option<(Command, SpawnLease)>> {
    build(program.as_ref(), None)
}

/// Supervise a direct `runc run` and ask the same owner to delete its private
/// container state before exiting on both normal completion and node crash.
pub fn runc_command(
    program: impl AsRef<OsStr>,
    root: &Path,
    id: &str,
) -> Result<Option<(Command, SpawnLease)>> {
    build(
        program.as_ref(),
        Some(RuncCleanup {
            root: root.to_path_buf(),
            id: id.to_string(),
        }),
    )
}

fn build(program: &OsStr, cleanup: Option<RuncCleanup>) -> Result<Option<(Command, SpawnLease)>> {
    let Some(binary) = SUPERVISOR_BINARY.get() else {
        return Ok(None);
    };
    anyhow::ensure!(!program.is_empty(), "supervised program is empty");
    let lease = SpawnLease::new()?;
    let read_fd = lease.read.as_raw_fd();
    let write_fd = lease.write.as_raw_fd();
    let parent = std::process::id() as libc::pid_t;
    let mut command = Command::new(binary);
    command.arg("internal-process-supervisor");
    if let Some(cleanup) = cleanup {
        command
            .arg("--runc-root")
            .arg(cleanup.root)
            .arg("--runc-id")
            .arg(cleanup.id);
    }
    command.arg("--").arg(program);
    command.process_group(0);
    unsafe {
        command.pre_exec(move || {
            install_lease_fd(read_fd, write_fd)?;
            // Parent death closes the node-owned lease write end, which is
            // the stop authority. SIGCONT only guarantees a deliberately
            // stopped supervisor can run its EOF cleanup before it exits.
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGCONT) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                return Err(std::io::Error::other(
                    "node exited before supervisor startup",
                ));
            }
            Ok(())
        });
    }
    Ok(Some((command, lease)))
}

fn install_lease_fd(read_fd: libc::c_int, write_fd: libc::c_int) -> std::io::Result<()> {
    if read_fd == LEASE_FD {
        if unsafe { libc::fcntl(LEASE_FD, libc::F_SETFD, 0) } == -1 {
            return Err(std::io::Error::last_os_error());
        }
    } else {
        if unsafe { libc::dup2(read_fd, LEASE_FD) } == -1 {
            return Err(std::io::Error::last_os_error());
        }
        unsafe { libc::close(read_fd) };
    }
    unsafe { libc::close(write_fd) };
    Ok(())
}

pub struct SpawnLease {
    read: OwnedFd,
    write: OwnedFd,
}

impl SpawnLease {
    fn new() -> Result<Self> {
        let mut fds = [-1; 2];
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Self {
            read: unsafe { OwnedFd::from_raw_fd(fds[0]) },
            write: unsafe { OwnedFd::from_raw_fd(fds[1]) },
        })
    }

    pub fn spawned(self, pid: Option<u32>) -> Result<OwnedSupervisor> {
        let pid = pid.context("supervisor child has no pid")?;
        let signal = SignalTarget::open(pid)?;
        tracker::register(&signal);
        drop(self.read);
        Ok(OwnedSupervisor {
            signal,
            lease: Some(self.write),
            armed: true,
        })
    }
}

pub struct OwnedSupervisor {
    signal: SignalTarget,
    lease: Option<OwnedFd>,
    armed: bool,
}

impl OwnedSupervisor {
    pub fn signal_target(&self) -> Result<SignalTarget> {
        self.signal.try_clone()
    }

    pub fn terminate(&mut self) {
        if !self.armed {
            return;
        }
        self.armed = false;
        // Closing the node-only lease is the stop request. SIGCONT carries no
        // termination meaning; it only lets a deliberately stopped owner run
        // its lease watcher and descendant cleanup.
        drop(self.lease.take());
        let _ = self.signal.signal(libc::SIGCONT);
    }
}

impl Drop for OwnedSupervisor {
    fn drop(&mut self) {
        self.terminate();
    }
}

//! Linux backend: build the commands that run below the hidden supervisor.

use super::{identity, pidfd::SignalTarget, supervisor::RuncCleanup, tracker};
use anyhow::{Context, Result};
use opencoder_core::platform::RunAsIdentity;
use std::{
    ffi::{OsStr, OsString},
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
/// `run_as` asks the supervisor to drop the workload to that identity before
/// exec; other library embedders retain the existing direct-process behavior.
pub fn command(
    program: impl AsRef<OsStr>,
    run_as: Option<&RunAsIdentity>,
) -> Result<Option<(Command, SpawnLease)>> {
    build(program.as_ref(), None, run_as)
}

/// Supervise a direct `runc run` and ask the same owner to delete its private
/// container state before exiting on both normal completion and node crash.
/// The DAG/runc chain never drops privileges here: its identity lives in the
/// container spec, which is why this constructor takes none.
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
        None,
    )
}

/// Supervisor argv prefix: binary, subcommand, cleanup flags, the raw-command
/// separator, identity flags (with their own separator) and the program.
/// Callers append the program's arguments afterwards.
fn supervisor_argv(
    binary: &Path,
    program: &OsStr,
    cleanup: Option<&RuncCleanup>,
    run_as: Option<&RunAsIdentity>,
) -> Result<Vec<OsString>> {
    anyhow::ensure!(!program.is_empty(), "supervised program is empty");
    let mut argv = vec![
        binary.as_os_str().to_os_string(),
        "internal-process-supervisor".into(),
    ];
    if let Some(cleanup) = cleanup {
        argv.push("--runc-root".into());
        argv.push(cleanup.root.as_os_str().to_os_string());
        argv.push("--runc-id".into());
        argv.push(OsString::from(&cleanup.id));
    }
    argv.push(identity::SEPARATOR.into());
    if let Some(identity) = run_as {
        identity::validate(identity)?;
        argv.extend(identity::flags(identity));
        argv.push(identity::SEPARATOR.into());
    }
    argv.push(program.to_os_string());
    Ok(argv)
}

fn build(
    program: &OsStr,
    cleanup: Option<RuncCleanup>,
    run_as: Option<&RunAsIdentity>,
) -> Result<Option<(Command, SpawnLease)>> {
    let Some(binary) = SUPERVISOR_BINARY.get() else {
        return Ok(None);
    };
    let argv = supervisor_argv(binary, program, cleanup.as_ref(), run_as)?;
    let lease = SpawnLease::new()?;
    let read_fd = lease.read.as_raw_fd();
    let write_fd = lease.write.as_raw_fd();
    let parent = std::process::id() as libc::pid_t;
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(uid: u32, gid: u32, groups: Vec<u32>) -> RunAsIdentity {
        RunAsIdentity {
            uid,
            gid,
            groups,
            username: None,
        }
    }

    #[test]
    fn argv_appends_identity_flags_between_separators() {
        let argv = supervisor_argv(
            Path::new("/agent"),
            "bash".as_ref(),
            None,
            Some(&identity(950, 951, vec![960])),
        )
        .unwrap();
        assert_eq!(
            argv,
            [
                "/agent",
                "internal-process-supervisor",
                "--",
                "--uid",
                "950",
                "--gid",
                "951",
                "--groups",
                "960",
                "--",
                "bash"
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn argv_without_identity_keeps_the_legacy_shape() {
        let cleanup = RuncCleanup {
            root: Path::new("/state").into(),
            id: "container".into(),
        };
        let argv =
            supervisor_argv(Path::new("/agent"), "runc".as_ref(), Some(&cleanup), None).unwrap();
        assert_eq!(
            argv,
            [
                "/agent",
                "internal-process-supervisor",
                "--runc-root",
                "/state",
                "--runc-id",
                "container",
                "--",
                "runc"
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn argv_refuses_root_identities_and_empty_programs() {
        assert!(supervisor_argv(
            Path::new("/agent"),
            "bash".as_ref(),
            None,
            Some(&identity(0, 1, Vec::new()))
        )
        .is_err());
        assert!(supervisor_argv(
            Path::new("/agent"),
            "bash".as_ref(),
            None,
            Some(&identity(1, 0, Vec::new()))
        )
        .is_err());
        assert!(supervisor_argv(Path::new("/agent"), "".as_ref(), None, None).is_err());
    }

    #[test]
    fn argv_round_trips_through_the_supervisor_parser() {
        for run_as in [None, Some(identity(950, 951, vec![951, 960]))] {
            let argv = supervisor_argv(Path::new("/agent"), "bash".as_ref(), None, run_as.as_ref())
                .unwrap();
            // The binary CLI consumes the leading raw-command separator, so
            // the supervisor sees everything after argv[2].
            let parsed = super::super::supervisor_args(&argv[3..]).unwrap();
            assert_eq!(parsed.program, "bash");
            assert!(parsed.args.is_empty());
            assert_eq!(parsed.identity, run_as);
        }
    }

    #[test]
    fn command_without_configured_supervisor_returns_none() {
        assert!(command("bash", Some(&identity(950, 951, Vec::new())))
            .unwrap()
            .is_none());
        assert!(command("bash", None).unwrap().is_none());
    }
}

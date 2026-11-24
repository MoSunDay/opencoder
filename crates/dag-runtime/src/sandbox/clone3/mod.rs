//! Native clone3 sandbox: PID/mount/UTS/IPC namespaces around the run's rootfs
//! overlay, without an OCI runtime. The node keeps owning the overlays; the
//! rootfs `dag-runner launch` applies the OCI config inside a private mount
//! namespace and parents the sandbox init, while `dag-runner exec` relays one
//! step into the live namespaces through `/proc/<init>`.
pub(crate) mod exec;
pub(crate) mod launch;
pub(crate) mod launcher;
mod mounts;
mod privilege;
mod process;

use anyhow::{ensure, Context, Result};
use opencoder_core::config::DagSandbox;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Launcher-written identity of the live sandbox, relative to the run root.
pub(crate) const INIT_RECORD: &str = "private/clone3/init.json";

/// Environment entry by which the launcher tells the init (see
/// `supervisor::init`) which descriptor carries the node's lease. Only the
/// launcher sets it: runc's init is told of no lease and never resolves one.
pub(crate) const LEASE_ENV: &str = "OPENCODER_SANDBOX_LEASE_FD";
/// Descriptor the launcher duplicates its stdin (the node's lease pipe) to
/// before exec'ing the init.
pub(crate) const LEASE_FD: i32 = 3;

/// Host pid pinned to one process incarnation (`/proc/<pid>/stat` starttime),
/// so a reused pid is never signalled or mistaken for the sandbox.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    pub pid: i32,
    pub start_time: u64,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub(crate) struct InitRecord {
    pub init: Identity,
    pub launcher: Identity,
}

/// Requirements of the pinned backend: `clone3` with `CLONE_NEWPID` (5.13+)
/// and root, because namespaces, chroot and mount are unprivileged-impossible.
pub fn supported() -> Result<()> {
    let release = kernel_release().context("read kernel release")?;
    requirements(release, unsafe { libc::geteuid() })
}

fn requirements(release: (u32, u32, u32), uid: u32) -> Result<()> {
    ensure!(
        (release.0, release.1) >= (5, 13),
        "clone3 sandbox requires Linux 5.13 or newer, found {}.{}.{}",
        release.0,
        release.1,
        release.2
    );
    ensure!(
        uid == 0,
        "clone3 sandbox requires the node to run as root (euid {uid})"
    );
    Ok(())
}

/// Backend a node actually uses once `dag.sandbox` is resolved.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    #[default]
    Runc,
    Clone3,
}

/// Resolve `dag.sandbox` into the backend this node uses for the next run.
/// `auto` prefers the native sandbox and falls back to runc; a pinned choice
/// never falls back.
pub fn select(config: &opencoder_core::Config) -> Result<Backend> {
    match config.dag.sandbox {
        DagSandbox::Auto => Ok(if supported().is_ok() {
            Backend::Clone3
        } else {
            Backend::Runc
        }),
        DagSandbox::Clone3 => {
            supported()?;
            Ok(Backend::Clone3)
        }
        DagSandbox::Runc => Ok(Backend::Runc),
    }
}

fn kernel_release() -> Result<(u32, u32, u32)> {
    let mut uts: libc::utsname = unsafe { std::mem::zeroed() };
    ensure!(
        unsafe { libc::uname(&mut uts) } == 0,
        "uname failed: {}",
        std::io::Error::last_os_error()
    );
    let release = uts.release;
    let bytes: Vec<u8> = release
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| *byte as u8)
        .collect();
    parse_release(std::str::from_utf8(&bytes).context("kernel release is not UTF-8")?)
        .context("unrecognized kernel release")
}

fn parse_release(release: &str) -> Option<(u32, u32, u32)> {
    let version = release.split('-').next()?;
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts
        .next()
        .map(|patch| patch.trim_end_matches(|c: char| !c.is_ascii_digit()))
        .and_then(|patch| patch.parse().ok())
        .unwrap_or(0);
    Some((major, minor, patch))
}

/// `starttime` (field 22) and liveness of a host pid; zombies count as gone.
fn parse_stat(stat: &str) -> Option<(u64, bool)> {
    let tail = stat.rsplit_once(") ")?.1;
    let mut fields = tail.split_whitespace();
    let alive = fields.next()? != "Z";
    let start_time = fields.nth(18)?.parse().ok()?;
    Some((start_time, alive))
}

pub(crate) fn identity(pid: i32) -> Option<Identity> {
    if pid <= 0 {
        return None;
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (start_time, alive) = parse_stat(&stat)?;
    alive.then_some(Identity { pid, start_time })
}

pub(crate) fn alive(who: Identity) -> bool {
    identity(who.pid) == Some(who)
}

/// Signal one exact process incarnation through a pidfd, so a pid reused
/// between the identity check and the signal is never hit; a process that is
/// already gone is not an error, any other failure is.
pub(crate) fn kill(who: Identity, signal: i32) -> Result<()> {
    // Safety: plain syscalls with scalar arguments; the descriptor is closed
    // on every path below.
    let pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, who.pid, 0) };
    if pidfd < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        // No pidfd for this process (descriptors exhausted, access refused):
        // the identity check alone guards the signal, as before.
        tracing::warn!(%error, pid = who.pid, "clone3 sandbox pidfd unavailable; signalling by identity");
        return kill_by_identity(who, signal);
    }
    let pidfd = pidfd as i32;
    // The descriptor pins the process; the starttime read afterwards tells
    // whether it is still the recorded incarnation.
    let failure = unsafe {
        let result = if identity(who.pid) == Some(who) {
            libc::syscall(libc::SYS_pidfd_send_signal, pidfd, signal, 0, 0)
        } else {
            0
        };
        let failure = (result != 0).then(std::io::Error::last_os_error);
        libc::close(pidfd);
        failure
    };
    match failure {
        // Gone before or while being signalled.
        Some(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(()),
        None => Ok(()),
        Some(error) => Err(error).context("signal clone3 sandbox process"),
    }
}

/// Signal guarded by the identity check alone (no pidfd available).
fn kill_by_identity(who: Identity, signal: i32) -> Result<()> {
    if !alive(who) {
        return Ok(());
    }
    if unsafe { libc::kill(who.pid, signal) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error).context("signal clone3 sandbox process");
        }
    }
    Ok(())
}

/// Read the launcher-written record; a missing or stale record is `None`.
pub(crate) fn init_record(root: &Path) -> Option<InitRecord> {
    let record: InitRecord =
        serde_json::from_slice(&std::fs::read(root.join(INIT_RECORD)).ok()?).ok()?;
    (alive(record.init) || alive(record.launcher)).then_some(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_release_versions_parse_like_uname() {
        assert_eq!(parse_release("6.8.0-58-generic"), Some((6, 8, 0)));
        assert_eq!(parse_release("5.15.0"), Some((5, 15, 0)));
        assert_eq!(parse_release("5.13"), Some((5, 13, 0)));
        assert_eq!(parse_release("4.18.0-553.el8.x86_64"), Some((4, 18, 0)));
        assert_eq!(parse_release("6.8.0rc7"), Some((6, 8, 0)));
        assert_eq!(parse_release("linux"), None);
        assert_eq!(parse_release("x.y.z"), None);
    }

    #[test]
    fn requirements_pin_the_pidfd_release_and_root() {
        assert!(requirements((6, 8, 0), 0).is_ok());
        assert!(requirements((5, 13, 0), 0).is_ok());
        let old = requirements((5, 10, 0), 0).unwrap_err();
        assert!(old.to_string().contains("5.13"));
        assert!(requirements((6, 8, 0), 1000)
            .unwrap_err()
            .to_string()
            .contains("root"));
    }

    #[test]
    fn auto_prefers_clone3_and_pinned_never_falls_back() {
        let mut config = opencoder_core::Config::default();
        config.dag.sandbox = DagSandbox::Runc;
        assert_eq!(select(&config).unwrap(), Backend::Runc);
        config.dag.sandbox = DagSandbox::Auto;
        let expected = if supported().is_ok() {
            Backend::Clone3
        } else {
            Backend::Runc
        };
        assert_eq!(select(&config).unwrap(), expected);
    }

    #[test]
    fn stat_parsing_skips_comm_with_spaces_and_parens() {
        let stat = "42 (dag (runner) x) S 1 42 42 0 -1 4194560 10 0 0 0 1 0 0 0 20 0 1 0 98765 100 7";
        assert_eq!(parse_stat(stat), Some((98765, true)));
        // starttime is field 22: the value after it (vsize) must not be mistaken for it.
        assert_ne!(parse_stat(stat), Some((100, true)));
        assert_eq!(parse_stat(&stat.replace(") S 1", ") Z 1")), Some((98765, false)));
        assert_eq!(parse_stat("invalid"), None);
        assert!(identity(0).is_none());
    }

    #[test]
    fn kill_reaches_the_recorded_incarnation_only() {
        use std::os::unix::process::ExitStatusExt;
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let who = identity(i32::try_from(child.id()).unwrap()).unwrap();
        // Another incarnation of this pid is never signalled.
        let stale = Identity {
            start_time: who.start_time.wrapping_add(1),
            ..who
        };
        kill(stale, libc::SIGKILL).unwrap();
        assert!(child.try_wait().unwrap().is_none());
        kill(who, libc::SIGKILL).unwrap();
        let status = child.wait().unwrap();
        assert_eq!(status.signal(), Some(libc::SIGKILL));
        // Gone: not an error, nothing to signal.
        kill(who, libc::SIGKILL).unwrap();
    }
}

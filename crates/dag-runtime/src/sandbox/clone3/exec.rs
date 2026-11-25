//! Step relay (`dag-runner exec <run_root> <key>`), run on the node as root by
//! `run::process::execute`. It joins the live sandbox through `/proc/<init>`
//! after verifying the init's identity, then forks the step inside the PID
//! namespace with the OCI process of `private/exec/<key>.json`. The relay's
//! stdin is the node's lease: when it closes, or the relay is signalled, the
//! step is stopped and the relay exits with the step's code.
use super::{
    launcher::{checked, errno, reap, report},
    process::{pointers, Process},
    InitRecord, INIT_RECORD,
};
use anyhow::{ensure, Context, Result};
use serde_json::Value;
use std::{
    ffi::CString,
    os::unix::{fs::OpenOptionsExt, io::AsRawFd},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

/// Grace period between the step's `SIGTERM` and `SIGKILL`.
const GRACE: Duration = Duration::from_secs(5);

/// Descriptors of the init's namespaces, all opened before the first
/// `setns`: entering the mount namespace changes what `/proc/<init>` is.
struct Namespaces {
    mounts: std::fs::File,
    uts: std::fs::File,
    ipc: std::fs::File,
    pid: std::fs::File,
    root: std::fs::File,
}

/// Relay body: `dag-runner exec <run_root> <key>`. Returns the step's exit code.
pub(crate) fn relay(root: &Path, key: &str) -> Result<i32> {
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "clone3 step relay must run as root"
    );
    let exec = root.join("private/exec");
    let process_file = exec.join(format!("{key}.json"));
    let spec: Value = serde_json::from_slice(&std::fs::read(&process_file)?)
        .with_context(|| format!("read {}", process_file.display()))?;
    let process = Process::parse(&spec)?;
    let record: InitRecord = serde_json::from_slice(&std::fs::read(root.join(INIT_RECORD))?)
        .context("clone3 sandbox is not running")?;
    ensure!(super::alive(record.init), "clone3 sandbox init is gone");
    let namespaces = Namespaces::open(record.init)?;
    // The descriptors pin the namespaces: make sure they still belong to the
    // init the record names before entering them.
    ensure!(super::alive(record.init), "clone3 sandbox init is gone");
    let null = std::fs::File::open("/dev/null").context("open /dev/null for the step")?;
    let argv = pointers(&process.argv);
    let env = pointers(&process.env);
    // Opened on the node: once the relay enters the sandbox root, the run
    // root's absolute path no longer resolves to the node's filesystem.
    let exec_dir =
        std::fs::File::open(&exec).with_context(|| format!("open {}", exec.display()))?;
    let pid_name = CString::new(format!("{key}.pid")).context("step key contains NUL")?;
    unsafe {
        namespaces.enter()?;
        let pid = libc::fork();
        if pid < 0 {
            return Err(std::io::Error::last_os_error()).context("fork clone3 step");
        }
        if pid == 0 {
            // Never returns to Rust: the step's process in the sandbox.
            child(null.as_raw_fd(), argv.as_ptr(), env.as_ptr(), &process);
        }
        let pidfd = libc::syscall(libc::SYS_pidfd_open, pid, 0) as i32;
        if pidfd < 0 {
            let _ = libc::kill(pid, libc::SIGKILL);
            let _ = reap(pid);
            return Err(std::io::Error::last_os_error()).context("open clone3 step pidfd");
        }
        write_pid(exec_dir.as_raw_fd(), &pid_name, pid)?;
        supervise(pidfd, pid)
    }
}

/// Records the step's host pid next to its process file, like `runc exec
/// --pid-file`, through a descriptor of the node's `private/exec` directory.
fn write_pid(dir: i32, name: &CString, pid: libc::pid_t) -> Result<()> {
    let mut bytes = [0u8; 12];
    let text = super::launcher::itoa(pid, &mut bytes);
    unsafe {
        let fd = libc::openat(
            dir,
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC,
            0o644,
        );
        if fd < 0 {
            return Err(std::io::Error::last_os_error()).context("write clone3 step pid file");
        }
        let written = libc::write(fd, text.as_ptr().cast(), text.len());
        let error = (written != text.len() as isize).then(std::io::Error::last_os_error);
        libc::close(fd);
        if let Some(error) = error {
            return Err(error).context("write clone3 step pid file");
        }
    }
    Ok(())
}

impl Namespaces {
    fn open(init: super::Identity) -> Result<Self> {
        let base = PathBuf::from(format!("/proc/{}", init.pid));
        let namespace = |name: &str| -> Result<std::fs::File> {
            std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_CLOEXEC)
                .open(base.join("ns").join(name))
                .with_context(|| format!("open sandbox {name} namespace of pid {}", init.pid))
        };
        let root = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY)
            .open(base.join("root"))
            .with_context(|| format!("open sandbox root of pid {}", init.pid))?;
        Ok(Self {
            mounts: namespace("mnt")?,
            uts: namespace("uts")?,
            ipc: namespace("ipc")?,
            pid: namespace("pid")?,
            root,
        })
    }

    /// Join the sandbox's namespaces; the PID namespace only takes effect for
    /// the children forked afterwards, so it is entered last.
    unsafe fn enter(self) -> Result<()> {
        // A relay inside a runtime thread must own its filesystem state.
        checked(
            libc::unshare(libc::CLONE_FS),
            "unshare step relay filesystem",
        )?;
        checked(
            libc::setns(self.mounts.as_raw_fd(), libc::CLONE_NEWNS),
            "enter sandbox mount namespace",
        )?;
        checked(libc::fchdir(self.root.as_raw_fd()), "enter sandbox root")?;
        checked(libc::chroot(c".".as_ptr()), "chroot into sandbox root")?;
        checked(libc::chdir(c"/".as_ptr()), "enter sandbox /")?;
        checked(
            libc::setns(self.uts.as_raw_fd(), libc::CLONE_NEWUTS),
            "enter sandbox UTS namespace",
        )?;
        checked(
            libc::setns(self.ipc.as_raw_fd(), libc::CLONE_NEWIPC),
            "enter sandbox IPC namespace",
        )?;
        checked(
            libc::setns(self.pid.as_raw_fd(), libc::CLONE_NEWPID),
            "enter sandbox PID namespace",
        )?;
        Ok(())
    }
}

/// Async-signal-safe: raw syscalls over memory resolved before `fork`.
unsafe fn child(
    null: i32,
    argv: *const *const libc::c_char,
    env: *const *const libc::c_char,
    process: &Process,
) -> ! {
    const STEPS: [&str; 3] = ["drop privileges", "enter cwd", "exec step"];
    libc::dup2(null, 0);
    let code = process.transition.apply();
    if code != 0 {
        report("step", STEPS[0], code);
    }
    if libc::chdir(process.cwd.as_ptr()) != 0 {
        report("step", STEPS[1], errno());
    }
    libc::execve(process.argv[0].as_ptr(), argv, env);
    report("step", STEPS[2], errno())
}

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn stop(_signal: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

/// Wait for the step, stopping it when the node's lease (stdin) closes or the
/// relay itself is asked to stop. Returns the step's exit code.
unsafe fn supervise(pidfd: i32, pid: libc::pid_t) -> Result<i32> {
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        libc::signal(signal, stop as *const () as libc::sighandler_t);
    }
    let mut polls = [
        libc::pollfd {
            fd: pidfd,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: 0,
            events: 0,
            revents: 0,
        },
    ];
    let mut stopping: Option<Instant> = None;
    let mut killed = false;
    loop {
        let lease_closed = polls[1].revents & (libc::POLLHUP | libc::POLLERR) != 0;
        if stopping.is_none() && (STOP.swap(false, Ordering::Relaxed) || lease_closed) {
            stopping = Some(Instant::now());
            let _ = libc::kill(pid, libc::SIGTERM);
        }
        let timeout = match stopping {
            Some(since) if !killed => {
                let remaining = GRACE.saturating_sub(since.elapsed());
                if remaining.is_zero() {
                    killed = true;
                    let _ = libc::kill(pid, libc::SIGKILL);
                    -1
                } else {
                    remaining.as_millis() as i32
                }
            }
            _ => -1,
        };
        if polls[0].revents & libc::POLLIN != 0 {
            break;
        }
        if libc::poll(polls.as_mut_ptr(), polls.len() as libc::nfds_t, timeout) < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EINTR) {
                libc::close(pidfd);
                return Err(error).context("wait for clone3 step");
            }
        }
    }
    libc::close(pidfd);
    Ok(reap(pid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grace_is_bounded_and_the_stop_flag_round_trips() {
        assert!(GRACE <= Duration::from_secs(10));
        assert!(!STOP.swap(true, Ordering::Relaxed));
        assert!(STOP.swap(false, Ordering::Relaxed));
    }
}

//! The launcher body (`dag-runner launch <run_root>`), run on the node as root
//! by `launch::spawn`. It owns the sandbox's mount namespace and parents its
//! init: unshare, apply the OCI mounts over the run's rootfs overlay, `clone3`
//! into a PID namespace, then the child chroots, mounts `proc` from inside
//! the namespace, masks paths, drops privileges and execs the init.
use super::{
    launch::diagnostic,
    mounts,
    process::{pointers, Process},
    InitRecord,
};
use anyhow::{bail, ensure, Context, Result};
use serde_json::Value;
use std::{
    ffi::CString,
    os::unix::{ffi::OsStrExt, io::AsRawFd},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

/// Everything the init needs, resolved before `clone3` so the child only
/// issues raw syscalls.
struct Plan {
    rootfs: PathBuf,
    rootfs_c: CString,
    mounts: Vec<mounts::Mount>,
    protection: mounts::Protection,
    proc: Option<(CString, u64)>,
    hostname: Option<String>,
    readonly_root: bool,
    process: Process,
}

/// Launcher body: `dag-runner launch <run_root>`. Returns the init's exit code.
pub(crate) fn launch(root: &Path) -> Result<i32> {
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "clone3 sandbox launcher must run as root"
    );
    let config: Value = serde_json::from_slice(&std::fs::read(root.join("bundle/config.json"))?)?;
    let mut stdin: libc::stat = unsafe { std::mem::zeroed() };
    checked(
        unsafe { libc::fstat(0, &mut stdin) },
        "inspect launcher stdin",
    )?;
    ensure!(
        (stdin.st_mode & libc::S_IFMT) == libc::S_IFIFO,
        "clone3 sandbox launcher stdin must be the node's lease pipe"
    );
    let rootfs = root.join("bundle/rootfs");
    ensure!(rootfs.is_dir(), "clone3 sandbox rootfs overlay is missing");
    Plan::prepare(&rootfs, &config)?.run(root)
}

impl Plan {
    fn prepare(rootfs: &Path, config: &Value) -> Result<Plan> {
        let mut process = Process::parse(&config["process"])?;
        process.lease(super::LEASE_FD);
        let all = mounts::parse_all(config)?;
        let proc = all
            .iter()
            .find(|mount| mount.kind == mounts::Kind::Proc)
            .map(|mount| {
                Ok::<_, anyhow::Error>((
                    mounts::target(Path::new("/"), &mount.destination)?,
                    mount.flags,
                ))
            })
            .transpose()?;
        let mounts: Vec<_> = all
            .into_iter()
            .filter(|mount| mount.kind != mounts::Kind::Proc)
            .collect();
        Ok(Self {
            rootfs: rootfs.to_path_buf(),
            rootfs_c: CString::new(rootfs.as_os_str().as_bytes())
                .context("run root contains NUL")?,
            mounts,
            protection: mounts::protection(config)?,
            proc,
            hostname: config["hostname"]
                .as_str()
                .filter(|host| !host.is_empty())
                .map(str::to_owned),
            readonly_root: config["root"]["readonly"].as_bool().unwrap_or(false),
            process,
        })
    }

    fn run(self, root: &Path) -> Result<i32> {
        unsafe {
            // CLONE_FS lets a launcher inside a runtime thread own its mounts.
            let namespaces =
                libc::CLONE_NEWNS | libc::CLONE_NEWUTS | libc::CLONE_NEWIPC | libc::CLONE_FS;
            checked(libc::unshare(namespaces), "unshare sandbox namespaces")?;
            let private = libc::MS_REC | libc::MS_PRIVATE;
            checked(
                libc::mount(
                    std::ptr::null(),
                    c"/".as_ptr(),
                    std::ptr::null(),
                    private,
                    std::ptr::null(),
                ),
                "make sandbox mounts private",
            )?;
            if let Some(host) = &self.hostname {
                let host = CString::new(host.as_str())?;
                checked(
                    libc::sethostname(host.as_ptr(), host.as_bytes().len()),
                    "set sandbox hostname",
                )?;
            }
        }
        if self.readonly_root {
            mounts::readonly_root(&self.rootfs)?;
        }
        for mount in &self.mounts {
            mounts::apply(&self.rootfs, mount)?;
            if mount.destination == Path::new("dev") && mount.kind == mounts::Kind::Tmpfs {
                mounts::devices(&self.rootfs)?;
            }
        }
        if let Some((target, _)) = &self.proc {
            // The chrooted child mounts proc from inside the PID namespace;
            // its mountpoint has to exist in the root already.
            let guest = target.to_string_lossy();
            let path = self.rootfs.join(guest.trim_start_matches('/'));
            if !path.exists() {
                std::fs::create_dir_all(&path).context("create proc mountpoint")?;
            }
        }
        self.clone_init(root)
    }

    fn clone_init(self, root: &Path) -> Result<i32> {
        let process = &self.process;
        let argv = pointers(&process.argv);
        let env = pointers(&process.env);
        let (ready_read, ready_write) = pipe()?;
        let null = std::fs::File::open("/dev/null").context("open /dev/null for sandbox init")?;
        let mut pidfd: libc::c_int = -1;
        let mut args = CloneArgs::new();
        args.flags = (libc::CLONE_NEWPID | libc::CLONE_PIDFD) as u64;
        args.pidfd = (&mut pidfd as *mut libc::c_int).cast::<u8>() as u64;
        args.exit_signal = libc::SIGCHLD as u64;
        let result = unsafe {
            libc::syscall(
                libc::SYS_clone3,
                &args as *const CloneArgs,
                std::mem::size_of::<CloneArgs>(),
            )
        };
        if result < 0 {
            return Err(std::io::Error::last_os_error()).context("clone3 sandbox init");
        }
        if result == 0 {
            // The child never returns to Rust: PID 1 of the new namespace.
            unsafe { self.child(ready_write, null.as_raw_fd(), argv.as_ptr(), env.as_ptr()) }
        }
        drop((ready_write, null));
        let pid = result as libc::pid_t;
        let init = super::identity(pid).context("read clone3 init identity")?;
        let mut ready = [0u8; READY.len()];
        let mut offset = 0;
        while offset < ready.len() {
            let count = unsafe {
                libc::read(
                    ready_read,
                    ready[offset..].as_mut_ptr().cast(),
                    ready.len() - offset,
                )
            };
            if count < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            if count <= 0 {
                let _ = reap(pid);
                bail!(
                    "clone3 sandbox init failed before exec: {}",
                    diagnostic(root)
                );
            }
            offset += count as usize;
        }
        let record = InitRecord {
            init,
            launcher: super::identity(std::process::id() as i32)
                .context("read launcher identity")?,
        };
        opencoder_core::atomic_write(
            &root.join(super::INIT_RECORD),
            &serde_json::to_vec(&record)?,
        )?;
        unsafe { supervise(pidfd, pid) }
    }

    /// Async-signal-safe: raw syscalls over memory resolved before `clone3`.
    /// Descriptor 3 keeps the node's lease (the launcher's stdin) so the init
    /// notices a node that goes away even when the launcher is already gone.
    unsafe fn child(
        &self,
        ready: i32,
        null: i32,
        argv: *const *const libc::c_char,
        env: *const *const libc::c_char,
    ) -> ! {
        const STEPS: [&str; 7] = [
            "enter rootfs",
            "mount proc",
            "protect proc",
            "report readiness",
            "drop privileges",
            "enter cwd",
            "exec init",
        ];
        if libc::dup2(0, super::LEASE_FD) != super::LEASE_FD {
            report("init", "duplicate lease", errno());
        }
        libc::dup2(null, 0);
        if libc::chdir(self.rootfs_c.as_ptr()) != 0
            || libc::chroot(c".".as_ptr()) != 0
            || libc::chdir(c"/".as_ptr()) != 0
        {
            report("init", STEPS[0], errno());
        }
        if let Some((target, flags)) = &self.proc {
            if libc::mount(
                c"proc".as_ptr(),
                target.as_ptr(),
                c"proc".as_ptr(),
                *flags,
                std::ptr::null(),
            ) != 0
            {
                report("init", STEPS[1], errno());
            }
        }
        let code = mounts::protect(&self.protection);
        if code != 0 {
            report("init", STEPS[2], code);
        }
        if libc::write(ready, READY.as_ptr() as *mut libc::c_void, READY.len())
            != READY.len() as isize
        {
            report("init", STEPS[3], errno());
        }
        let code = self.process.transition.apply();
        if code != 0 {
            report("init", STEPS[4], code);
        }
        if libc::chdir(self.process.cwd.as_ptr()) != 0 {
            report("init", STEPS[5], errno());
        }
        libc::execve(self.process.argv[0].as_ptr(), argv, env);
        report("init", STEPS[6], errno())
    }
}

pub(super) fn report(who: &str, step: &str, errno: i32) -> ! {
    let mut line = [0u8; 96];
    let mut len = 0;
    let mut append = |bytes: &[u8]| {
        for byte in bytes {
            if len < line.len() {
                line[len] = *byte;
                len += 1;
            }
        }
    };
    append(b"clone3 ");
    append(who.as_bytes());
    append(b": ");
    append(step.as_bytes());
    append(b" (errno ");
    append(itoa(errno, &mut [0u8; 12]));
    append(b")\n");
    unsafe {
        libc::write(2, line.as_ptr().cast(), len);
        libc::_exit(125)
    }
}

pub(super) fn itoa(value: i32, buffer: &mut [u8; 12]) -> &[u8] {
    let mut start = buffer.len();
    let negative = value < 0;
    let mut digits = value.unsigned_abs();
    loop {
        start -= 1;
        buffer[start] = b'0' + (digits % 10) as u8;
        digits /= 10;
        if digits == 0 {
            break;
        }
    }
    if negative {
        start -= 1;
        buffer[start] = b'-';
    }
    &buffer[start..]
}

/// Readiness marker the init writes once its root, proc and protection are in place.
const READY: &[u8] = b"ready";

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn stop(_signal: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

/// Wait for the init, killing it when the node's lease (stdin) closes or the
/// launcher itself is asked to stop. Returns the init's exit code.
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
    let mut stopping = false;
    loop {
        let lease_closed = polls[1].revents & (libc::POLLHUP | libc::POLLERR) != 0;
        if !stopping && (STOP.swap(false, Ordering::Relaxed) || lease_closed) {
            stopping = true;
            libc::kill(pid, libc::SIGKILL);
        }
        if polls[0].revents & libc::POLLIN != 0 {
            break;
        }
        if libc::poll(polls.as_mut_ptr(), polls.len() as libc::nfds_t, -1) < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EINTR) {
                return Err(error).context("wait for clone3 sandbox init");
            }
        }
    }
    Ok(reap(pid))
}

pub(super) fn reap(pid: libc::pid_t) -> i32 {
    let mut status = 0;
    loop {
        let result = unsafe { libc::waitpid(pid, &mut status, 0) };
        if result == pid {
            break;
        }
        if result < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return 1;
    }
    if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        1
    }
}

pub(super) fn errno() -> i32 {
    unsafe { *libc::__errno_location() }
}

pub(super) fn checked(result: i32, what: &str) -> Result<()> {
    if result != 0 {
        return Err(std::io::Error::last_os_error()).context(what.to_owned());
    }
    Ok(())
}

fn pipe() -> Result<(i32, i32)> {
    let mut fds = [0; 2];
    checked(
        unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) },
        "open sandbox pipe",
    )?;
    Ok((fds[0], fds[1]))
}

#[repr(C)]
struct CloneArgs {
    flags: u64,
    pidfd: u64,
    child_tid: u64,
    parent_tid: u64,
    exit_signal: u64,
    stack: u64,
    stack_size: u64,
    tls: u64,
}

impl CloneArgs {
    fn new() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_report_formats_negative_errno_without_allocating() {
        let mut buffer = [0u8; 12];
        assert_eq!(itoa(13, &mut buffer), b"13");
        assert_eq!(itoa(-1, &mut buffer), b"-1");
        assert_eq!(itoa(0, &mut buffer), b"0");
        assert_eq!(itoa(i32::MIN, &mut buffer), b"-2147483648");
        assert_eq!(std::mem::size_of::<CloneArgs>(), 64);
    }
}

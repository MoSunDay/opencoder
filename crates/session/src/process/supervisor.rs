//! Hidden agent process that owns and reaps one external workload tree.

use super::{pidfd::start_time, supervisor_args, SignalTarget, LEASE_FD};
use anyhow::{bail, Context, Result};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    ffi::OsString,
    io::Read,
    os::{fd::FromRawFd, unix::process::CommandExt},
    path::PathBuf,
    process::{Command, ExitStatus, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

static STOP: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug)]
pub struct RuncCleanup {
    pub root: PathBuf,
    pub id: String,
}

/// Run one command below a Linux child-subreaper and return the command's exit
/// code. The agent main exits with this code immediately after return.
pub fn supervisor_main(command: Vec<OsString>, cleanup: Option<RuncCleanup>) -> Result<i32> {
    STOP.store(false, Ordering::SeqCst);
    ensure_lease_fd()?;
    set_subreaper()?;
    install_stop_handlers();
    watch_node_lease();

    let (program, args) = supervisor_args(&command)?;
    let mut child_command = Command::new(program);
    child_command
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    child_command.process_group(0);
    let mut child = child_command.spawn().context("spawn supervised workload")?;
    let natural = wait_until_exit_or_stop(&mut child);
    terminate_descendants(std::process::id());
    let container = cleanup.as_ref().map(cleanup_runc).transpose();
    container?;
    let natural = natural?;
    Ok(natural.map(exit_code).unwrap_or(128 + libc::SIGKILL))
}

fn ensure_lease_fd() -> Result<()> {
    if unsafe { libc::fcntl(LEASE_FD, libc::F_GETFD) } == -1 {
        bail!("node process lease unavailable");
    }
    Ok(())
}

fn set_subreaper() -> Result<()> {
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) } != 0 {
        return Err(std::io::Error::last_os_error()).context("enable child subreaper");
    }
    Ok(())
}

extern "C" fn request_stop(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

fn install_stop_handlers() {
    unsafe {
        libc::signal(
            libc::SIGTERM,
            request_stop as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGINT,
            request_stop as *const () as libc::sighandler_t,
        );
    }
}

fn watch_node_lease() {
    std::thread::spawn(|| {
        let mut lease = unsafe { std::fs::File::from_raw_fd(LEASE_FD) };
        let mut byte = [0u8; 1];
        loop {
            match lease.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        STOP.store(true, Ordering::SeqCst);
    });
}

fn wait_until_exit_or_stop(child: &mut std::process::Child) -> Result<Option<ExitStatus>> {
    loop {
        if STOP.load(Ordering::SeqCst) {
            return Ok(None);
        }
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn exit_code(status: ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(libc::SIGKILL))
}

fn terminate_descendants(root: u32) {
    loop {
        let snapshots = match descendants(root) {
            Ok(snapshots) => snapshots,
            Err(_) => {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
        };
        if snapshots.is_empty() && reap_available().unwrap_or(false) {
            return;
        }
        let mut owned = Vec::with_capacity(snapshots.len());
        for (pid, expected_start) in snapshots {
            let Ok(target) = SignalTarget::open(pid) else {
                continue;
            };
            if start_time(pid) != Some(expected_start) {
                continue;
            }
            if target.signal(libc::SIGSTOP).is_err() {
                continue;
            }
            owned.push(target);
        }
        for target in &owned {
            let _ = target.signal(libc::SIGKILL);
        }
        let _ = reap_available();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn descendants(root: u32) -> Result<BTreeMap<u32, u64>> {
    let mut queue = VecDeque::from([root]);
    let mut seen = BTreeSet::from([root]);
    let mut descendants = BTreeMap::new();
    while let Some(parent) = queue.pop_front() {
        for child in direct_children(parent)? {
            if !seen.insert(child) {
                continue;
            }
            if let Some(start) = start_time(child) {
                descendants.insert(child, start);
                queue.push_back(child);
            }
        }
    }
    Ok(descendants)
}

fn direct_children(pid: u32) -> Result<Vec<u32>> {
    let tasks = match std::fs::read_dir(format!("/proc/{pid}/task")) {
        Ok(tasks) => tasks,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut children = BTreeSet::new();
    for task in tasks {
        let task = task?;
        let path = task.path().join("children");
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for value in text.split_whitespace() {
            children.insert(value.parse::<u32>()?);
        }
    }
    Ok(children.into_iter().collect())
}

fn reap_available() -> Result<bool> {
    loop {
        let result = unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) };
        if result > 0 {
            continue;
        }
        if result == -1 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ECHILD) {
                return Ok(true);
            }
            return Err(error.into());
        }
        return Ok(false);
    }
}

fn cleanup_runc(cleanup: &RuncCleanup) -> Result<()> {
    if !cleanup.root.join(&cleanup.id).exists() {
        return Ok(());
    }
    let mut child = Command::new("runc")
        .arg("--root")
        .arg(&cleanup.root)
        .args(["delete", "--force", &cleanup.id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn supervised runc cleanup")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait()? {
            anyhow::ensure!(
                status.success() || !cleanup.root.join(&cleanup.id).exists(),
                "supervised runc cleanup failed: {status}"
            );
            break;
        }
        if Instant::now() >= deadline {
            child.kill()?;
            let _ = child.wait();
            bail!("supervised runc cleanup exceeded 5s");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    anyhow::ensure!(
        !cleanup.root.join(&cleanup.id).exists(),
        "supervised runc state remains after cleanup"
    );
    Ok(())
}

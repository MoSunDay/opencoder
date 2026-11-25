use anyhow::{ensure, Context, Result};
use std::{
    collections::BTreeSet,
    os::unix::{io::FromRawFd, process::CommandExt},
    path::Path,
    time::{Duration, Instant},
};
use tokio::{
    process::Command,
    signal::unix::{signal, SignalKind},
};

pub async fn run(arguments: Vec<String>) -> Result<i32> {
    match arguments.first().map(String::as_str) {
        Some("init") if arguments.len() == 1 => init().await,
        Some("check") if arguments.len() == 2 => {
            ensure!(
                std::fs::read_to_string("/run/opencoder-private/identity")? == arguments[1],
                "DAG container identity mismatch"
            );
            Ok(0)
        }
        Some("step") if arguments.len() > 1 => step(&arguments[1..]).await,
        Some("probe") if arguments.len() == 1 => {
            ensure!(
                !std::fs::read_to_string("/run/opencoder-private/identity")?.is_empty(),
                "DAG identity is missing"
            );
            println!("native DAG probe completed");
            Ok(0)
        }
        // Node-side clone3 entry points (see `clone3`): both block their thread
        // in poll(2); this runtime has nothing else to do meanwhile.
        Some("launch") if arguments.len() == 2 => {
            super::clone3::launcher::launch(Path::new(&arguments[1]))
        }
        Some("exec") if arguments.len() == 3 => {
            super::clone3::exec::relay(Path::new(&arguments[1]), &arguments[2])
        }
        _ => anyhow::bail!("dag-runner requires init, step <program> [args], launch or exec"),
    }
}

async fn init() -> Result<i32> {
    let mut terminate = signal(SignalKind::terminate())?;
    let mut children = signal(SignalKind::child())?;
    let lease = lease_closed();
    tokio::pin!(lease);
    loop {
        tokio::select! {
            _ = terminate.recv() => return Ok(0),
            _ = children.recv() => { reap()?; }
            _ = &mut lease => return Ok(0),
        }
    }
}

/// The node's lease: the descriptor the clone3 launcher names in the
/// environment (`clone3::LEASE_ENV`, see `clone3::launcher`); EOF means the
/// node is gone and the sandbox must not outlive it, whatever happened to the
/// launcher. runc's init is told of no lease, so this never resolves there,
/// whatever its inherited descriptors hold.
async fn lease_closed() {
    let Some(fd) = lease_fd() else {
        return std::future::pending::<()>().await;
    };
    let closed = tokio::task::spawn_blocking(move || {
        // A named lease that is not open leaves the sandbox unowned: stop.
        if unsafe { libc::fcntl(fd, libc::F_GETFD) } == -1 {
            return true;
        }
        // Safety: the descriptor is the lease pipe the launcher duplicated;
        // this `File` is its only reader from here on.
        let mut lease = unsafe { std::fs::File::from_raw_fd(fd) };
        lease_ended(&mut lease)
    })
    .await;
    if !matches!(closed, Ok(true)) {
        std::future::pending::<()>().await;
    }
}

/// Only the lease's EOF (or a lease that cannot be read) ends the watch:
/// bytes written to it and interrupted reads are consumed, so an open lease
/// is never left unwatched.
fn lease_ended(lease: &mut impl std::io::Read) -> bool {
    let mut byte = [0u8; 1];
    loop {
        match lease.read(&mut byte) {
            Ok(0) => return true,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return true,
        }
    }
}

/// Descriptor the launcher named as the lease, if any.
fn lease_fd() -> Option<i32> {
    std::env::var(super::clone3::LEASE_ENV)
        .ok()?
        .parse::<i32>()
        .ok()
        .filter(|fd| *fd > 2)
}

async fn step(arguments: &[String]) -> Result<i32> {
    ensure!(
        unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } == 0,
        "enable step child supervision"
    );
    unsafe {
        libc::umask(0o002);
    }
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut command = Command::new(&arguments[0]);
    command.args(&arguments[1..]);
    command.as_std_mut().process_group(0);
    let mut child = command.spawn().context("spawn native step program")?;
    let status = tokio::select! {
        status = child.wait() => Some(status?),
        _ = terminate.recv() => None,
        _ = interrupt.recv() => None,
    };
    let code = if let Some(status) = status {
        status.code().unwrap_or(1)
    } else {
        if let Some(pid) = child.id() {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGTERM);
            }
        }
        match tokio::time::timeout(Duration::from_secs(1), child.wait()).await {
            Ok(status) => {
                status?;
            }
            Err(_) => {
                if let Some(pid) = child.id() {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
                child.wait().await?;
            }
        }
        143
    };
    let started = Instant::now();
    loop {
        let descendants = descendants(std::process::id() as i32)?;
        for pid in &descendants {
            unsafe {
                libc::kill(*pid, libc::SIGKILL);
            }
        }
        let pending = reap()?;
        if descendants.is_empty() && !pending {
            return Ok(code);
        }
        ensure!(
            started.elapsed() < Duration::from_secs(5),
            "step descendants did not terminate"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn reap() -> Result<bool> {
    loop {
        let result = unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) };
        match result {
            0 => return Ok(true),
            -1 => {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ECHILD) {
                    return Ok(false);
                }
                if error.raw_os_error() != Some(libc::EINTR) {
                    return Err(error).context("reap step descendant");
                }
            }
            _ => {}
        }
    }
}

fn process_parent(stat: &str) -> Option<(i32, i32)> {
    let pid = stat.split_once(' ')?.0.parse().ok()?;
    let tail = stat.rsplit_once(") ")?.1;
    let parent = tail.split_whitespace().nth(1)?.parse().ok()?;
    Some((pid, parent))
}

fn descendants(owner: i32) -> Result<BTreeSet<i32>> {
    let processes: Vec<_> = std::fs::read_dir("/proc")?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            entry.file_name().to_str()?.parse::<i32>().ok()?;
            process_parent(&std::fs::read_to_string(entry.path().join("stat")).ok()?)
        })
        .collect();
    let mut owned = BTreeSet::from([owner]);
    loop {
        let before = owned.len();
        for (pid, parent) in &processes {
            if owned.contains(parent) {
                owned.insert(*pid);
            }
        }
        if before == owned.len() {
            break;
        }
    }
    owned.remove(&owner);
    Ok(owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{ErrorKind, Read, Result};

    #[test]
    fn process_identity_handles_spaces_and_parentheses() {
        assert_eq!(
            process_parent("123 (name with ) spaces) S 42 0 0"),
            Some((123, 42))
        );
        assert_eq!(process_parent("invalid"), None);
    }

    /// Reader replaying its scripted results (last first), then failing:
    /// the watch must have ended before the script runs out.
    struct Scripted(Vec<Result<usize>>);

    impl Read for Scripted {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
            match self.0.pop() {
                Some(Ok(count)) => Ok(count.min(buf.len())),
                Some(Err(error)) => Err(error),
                None => panic!("lease read past the scripted results"),
            }
        }
    }

    #[test]
    fn lease_watch_ends_only_at_eof_or_on_an_unreadable_lease() {
        assert!(lease_ended(&mut Scripted(vec![Ok(0)])));
        // Bytes on the lease and an interrupted read are drained, not its end.
        assert!(lease_ended(&mut Scripted(vec![
            Ok(0),
            Err(ErrorKind::Interrupted.into()),
            Ok(1),
            Ok(1),
        ])));
        // A lease that cannot be read leaves the sandbox unowned: stop now.
        assert!(lease_ended(&mut Scripted(vec![Err(
            ErrorKind::BrokenPipe.into()
        )])));
    }
}

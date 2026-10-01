use anyhow::{ensure, Context, Result};
use std::{
    collections::BTreeSet,
    os::unix::process::CommandExt,
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
        _ => anyhow::bail!("dag-runner requires init or step <program> [args]"),
    }
}

async fn init() -> Result<i32> {
    let mut terminate = signal(SignalKind::terminate())?;
    let mut children = signal(SignalKind::child())?;
    loop {
        tokio::select! {
            _ = terminate.recv() => return Ok(0),
            _ = children.recv() => { reap()?; }
        }
    }
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

    #[test]
    fn process_identity_handles_spaces_and_parentheses() {
        assert_eq!(
            process_parent("123 (name with ) spaces) S 42 0 0"),
            Some((123, 42))
        );
        assert_eq!(process_parent("invalid"), None);
    }
}

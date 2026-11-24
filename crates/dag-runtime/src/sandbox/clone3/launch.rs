//! Node-side handle of a live clone3 sandbox. `RunContainer::start` spawns the
//! rootfs `dag-runner launch` (see `launcher.rs`) and keeps its stdin as the
//! lease: when the lease closes the launcher kills the init, so the sandbox
//! cannot outlive its owner.
use anyhow::{bail, ensure, Context, Result};
use std::{
    path::Path,
    process::Stdio,
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};
use tokio::process::{Child, Command};

/// Node-side handle: the launcher process and, through it, the sandbox init.
pub(crate) struct Launcher {
    pub init: super::Identity,
    child: Mutex<Option<Child>>,
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Spawn the launcher and wait until the init has entered its root.
pub(crate) async fn spawn(root: &Path, runner: &Path, log: std::fs::File) -> Result<Launcher> {
    let record = root.join(super::INIT_RECORD);
    match std::fs::remove_file(&record) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("clear stale clone3 sandbox record"),
    }
    let mut child = Command::new(runner)
        .arg("launch")
        .arg(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(log)
        .kill_on_drop(true)
        .spawn()
        .context("start clone3 sandbox launcher")?;
    let started = Instant::now();
    let record = loop {
        if let Some(record) = super::init_record(root) {
            break record;
        }
        if let Some(status) = child.try_wait()? {
            bail!(
                "clone3 sandbox launcher exited with {status}: {}",
                diagnostic(root)
            );
        }
        ensure!(
            started.elapsed() < Duration::from_secs(30),
            "clone3 sandbox start timed out: {}",
            diagnostic(root)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    ensure!(
        super::alive(record.init),
        "clone3 sandbox init is not running: {}",
        diagnostic(root)
    );
    Ok(Launcher {
        init: record.init,
        child: Mutex::new(Some(child)),
    })
}

pub(super) fn diagnostic(root: &Path) -> String {
    std::fs::read(root.join("private/start.log"))
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
        .unwrap_or_default()
}

/// Stop a sandbox whose owner is gone: kill the init, then wait for both the
/// init and the launcher to leave before mounts are released.
pub(crate) async fn stop_abandoned(root: &Path) -> Result<()> {
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || stop_abandoned_blocking(&root))
        .await
        .context("clone3 abandoned cleanup task failed")?
}

/// Blocking form for callers that already run off the runtime (`Drop`).
pub(crate) fn stop_abandoned_blocking(root: &Path) -> Result<()> {
    let Some(record) = super::init_record(root) else {
        return Ok(());
    };
    super::kill(record.init, libc::SIGKILL)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while super::alive(record.init) || super::alive(record.launcher) {
        ensure!(
            Instant::now() < deadline,
            "clone3 sandbox processes did not exit after cleanup"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

impl Launcher {
    /// Closing the lease makes the launcher kill the init and exit once it has
    /// reaped it; a launcher that ignores the lease is killed, after which the
    /// init sees its own lease close. Both must be gone before unmounting.
    pub(crate) async fn stop(&self) -> Result<()> {
        if let Some(child) = locked(&self.child).as_mut() {
            child.stdin.take();
        }
        let exited = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let status: Option<std::process::ExitStatus> = {
                    let mut guard = locked(&self.child);
                    match guard.as_mut() {
                        Some(child) => child.try_wait()?,
                        None => return Ok::<_, anyhow::Error>(()),
                    }
                };
                if status.is_some() {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        if exited.is_err() {
            if let Some(child) = locked(&self.child).as_mut() {
                child.start_kill().context("kill clone3 sandbox launcher")?;
            }
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while super::alive(self.init) {
            ensure!(
                Instant::now() < deadline,
                "clone3 sandbox init did not exit after its lease closed"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(())
    }
}

impl Drop for Launcher {
    fn drop(&mut self) {
        let guard = self.child.get_mut().unwrap_or_else(PoisonError::into_inner);
        if let Some(child) = guard.as_mut() {
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.start_kill();
            }
        }
    }
}


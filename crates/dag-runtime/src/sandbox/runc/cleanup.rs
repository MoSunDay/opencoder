//! Reap each delete command and confirm removal before releasing its owner.
use anyhow::{ensure, Context, Result};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    process::{Child, Command},
    time::timeout,
};

pub(crate) async fn delete_force(root: &Path, id: &str) -> Result<()> {
    delete_using(&root.join(id), Duration::from_secs(5), || {
        Command::new("runc")
            .arg("--root")
            .arg(root)
            .args(["delete", "--force", id])
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("spawn runc cleanup")
    })
    .await
}

async fn delete_using(
    state: &Path,
    warn_after: Duration,
    mut launch: impl FnMut() -> Result<Child>,
) -> Result<()> {
    for attempt in 0..2 {
        if opencoder_session::process::remove_empty_runc_state(state)? {
            return Ok(());
        }
        match finish_delete(launch()?, state, warn_after).await {
            Ok(()) => return Ok(()),
            Err(error) if attempt == 0 => {
                tracing::warn!(%error, path = %state.display(), "retrying owned runc cleanup");
                // runc itself has a ten-second init-exit deadline. A killed
                // init can still be finishing kernel writeback after that.
                // Wait for that exact process identity before retrying delete.
                super::init_exit::wait(state)
                    .await
                    .context("wait for owned container init after failed delete")?;
            }
            Err(error) => return Err(error).context("runc cleanup failed after retry"),
        }
    }
    unreachable!("both cleanup attempts return or report failure")
}

async fn finish_delete(mut child: Child, state: &Path, warn_after: Duration) -> Result<()> {
    let status = match timeout(warn_after, child.wait()).await {
        Ok(status) => status?,
        Err(_) => {
            // Deleting the mount namespace may wait for OverlayFS writeback.
            // Killing runc here interrupts valid cleanup and cannot cancel
            // that kernel work. Retain ownership until the actual exit, as
            // we already do for the following unmount operation.
            tracing::warn!(path = %state.display(), elapsed_seconds = warn_after.as_secs_f64(),
                "waiting for owned runc cleanup to finish");
            child.wait().await.context("wait for slow runc cleanup")?
        }
    };
    let removed = opencoder_session::process::remove_empty_runc_state(state)?;
    ensure!(status.success() || removed, "runc delete failed: {status}");
    ensure!(
        !state.exists(),
        "runc container state remains after cleanup"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delete_child(path: &Path) -> Child {
        Command::new("sh")
            .args([
                "-c",
                "rm -- \"$1/state.json\" && rmdir -- \"$1\"",
                "cleanup",
            ])
            .arg(path)
            .kill_on_drop(true)
            .spawn()
            .unwrap()
    }

    fn state(root: &Path) -> std::path::PathBuf {
        let path = root.join("owned-container");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(
            path.join("state.json"),
            br#"{"id":"owned-container","init_process_pid":0,"init_process_start":0}"#,
        )
        .unwrap();
        path
    }

    #[tokio::test]
    async fn slow_delete_keeps_ownership_until_reaped_and_removed() {
        use tokio::io::AsyncWriteExt;
        let root = tempfile::tempdir().unwrap();
        let path = state(root.path());
        let mut child = Command::new("sh")
            .args([
                "-c",
                "read reply; rm -- \"$1/state.json\" && rmdir -- \"$1\"",
                "cleanup",
            ])
            .arg(&path)
            .stdin(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut gate = child.stdin.take().unwrap();
        let pid = child.id().unwrap();
        let owned = path.clone();
        let mut cleanup =
            tokio::spawn(
                async move { finish_delete(child, &owned, Duration::from_millis(1)).await },
            );
        assert!(timeout(Duration::from_millis(20), &mut cleanup)
            .await
            .is_err());
        assert!(!cleanup.is_finished());
        assert!(path.join("state.json").exists());
        assert!(Path::new(&format!("/proc/{pid}")).exists());
        gate.write_all(b"finish\n").await.unwrap();
        drop(gate);
        cleanup.await.unwrap().unwrap();
        assert!(!path.exists());
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
    }

    #[tokio::test]
    async fn failed_delete_retries_same_owned_state_before_success() {
        let root = tempfile::tempdir().unwrap();
        let path = state(root.path());
        let mut attempts = 0;
        delete_using(&path, Duration::from_secs(1), || {
            attempts += 1;
            if attempts == 1 {
                return Command::new("sh")
                    .args(["-c", "exit 1"])
                    .spawn()
                    .map_err(Into::into);
            }
            Ok(delete_child(&path))
        })
        .await
        .unwrap();
        assert_eq!(attempts, 2);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn failed_delete_retains_owned_state_after_both_reaped_attempts() {
        let root = tempfile::tempdir().unwrap();
        let path = state(root.path());
        let mut pids = Vec::new();
        let error = delete_using(&path, Duration::from_millis(20), || {
            let child = Command::new("sh").args(["-c", "exit 1"]).spawn().unwrap();
            pids.push(child.id().unwrap());
            Ok(child)
        })
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("runc delete failed"));
        assert_eq!(pids.len(), 2);
        assert!(pids
            .iter()
            .all(|pid| !Path::new(&format!("/proc/{pid}")).exists()));
        assert_eq!(
            std::fs::read(path.join("state.json")).unwrap(),
            br#"{"id":"owned-container","init_process_pid":0,"init_process_start":0}"#
        );
    }
}

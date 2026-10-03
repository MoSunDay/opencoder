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
    budget: Duration,
    mut launch: impl FnMut() -> Result<Child>,
) -> Result<()> {
    for attempt in 0..2 {
        if opencoder_session::process::remove_empty_runc_state(state)? {
            return Ok(());
        }
        match finish_delete(launch()?, state, budget).await {
            Ok(()) => return Ok(()),
            Err(error) if attempt == 0 => {
                tracing::warn!(%error, path = %state.display(), "retrying owned runc cleanup");
            }
            Err(error) => return Err(error).context("runc cleanup failed after retry"),
        }
    }
    unreachable!("both cleanup attempts return or report failure")
}

async fn finish_delete(mut child: Child, state: &Path, budget: Duration) -> Result<()> {
    let status = match timeout(budget, child.wait()).await {
        Ok(status) => status?,
        Err(_) => {
            child.kill().await.context("reap timed-out runc cleanup")?;
            // Completion can win the timer while its wait notification is
            // delayed. The reaped command and absent state prove removal.
            ensure!(
                opencoder_session::process::remove_empty_runc_state(state)?,
                "runc delete exceeded {}s",
                budget.as_secs_f64()
            );
            return Ok(());
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

    fn sleeping_child() -> Child {
        Command::new("sh")
            .args(["-c", "exec sleep 10"])
            .kill_on_drop(true)
            .spawn()
            .unwrap()
    }

    fn state(root: &Path) -> std::path::PathBuf {
        let path = root.join("owned-container");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("state.json"), b"owned state").unwrap();
        path
    }

    #[tokio::test]
    async fn late_delete_reply_succeeds_only_after_reaping_and_removed_state() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("removed-container");
        let child = sleeping_child();
        let pid = child.id().unwrap();
        finish_delete(child, &path, Duration::from_millis(20))
            .await
            .unwrap();
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
    }

    #[tokio::test]
    async fn timed_out_delete_retries_same_owned_state_before_success() {
        let root = tempfile::tempdir().unwrap();
        let path = state(root.path());
        let mut attempts = 0;
        delete_using(&path, Duration::from_secs(1), || {
            attempts += 1;
            if attempts == 1 {
                return Ok(sleeping_child());
            }
            Command::new("sh")
                .args([
                    "-c",
                    "rm -- \"$1/state.json\" && rmdir -- \"$1\"",
                    "cleanup",
                ])
                .arg(&path)
                .kill_on_drop(true)
                .spawn()
                .map_err(Into::into)
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
            let child = sleeping_child();
            pids.push(child.id().unwrap());
            Ok(child)
        })
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("runc delete exceeded"));
        assert_eq!(pids.len(), 2);
        assert!(pids
            .iter()
            .all(|pid| !Path::new(&format!("/proc/{pid}")).exists()));
        assert_eq!(
            std::fs::read(path.join("state.json")).unwrap(),
            b"owned state"
        );
    }
}

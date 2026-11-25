//! Owned runc lifecycle: cancellation/timeout stops the container and
//! reaps the launcher before returning; cleanup failures remain visible.
use anyhow::{Context, Result};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

mod cleanup;
mod init_exit;
mod recovery;
pub(super) use cleanup::delete_force;
pub use recovery::cleanup_owned_containers;

pub fn runc_available() -> bool {
    std::process::Command::new("runc")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub async fn run_step_cancellable(
    bundle_dir: &Path,
    id: &str,
    timeout_secs: Option<u64>,
    cancel: CancellationToken,
) -> Result<(i32, String)> {
    run_step_streamed(bundle_dir, id, timeout_secs, cancel, None).await
}

/// [`run_step_cancellable`] with the container's piped stdout/stderr mirrored
/// into `output` (the node-store step log) AS THEY ARE READ, so a remote
/// console follows a long-running sandboxed step live. The bounded collectors
/// below still own the returned text and the `output_limit_exceeded` cap; the
/// tee only observes the same bytes.
pub async fn run_step_streamed(
    bundle_dir: &Path,
    id: &str,
    timeout_secs: Option<u64>,
    cancel: CancellationToken,
    output: Option<crate::step_log::StepOutputLog>,
) -> Result<(i32, String)> {
    anyhow::ensure!(
        !id.is_empty()
            && id.len() <= 255
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "invalid container id"
    );
    anyhow::ensure!(!cancel.is_cancelled(), "runc step cancelled");
    // A private state root prevents unrelated runtimes or test runs from
    // colliding with this execution's container name.
    let root = bundle_dir.join("runc-state");
    std::fs::create_dir_all(&root)?;
    let supervised = opencoder_session::process::runc_command("runc", &root, id)?;
    let (mut command, lease) = match supervised {
        Some((command, lease)) => (command, Some(lease)),
        None => (Command::new("runc"), None),
    };
    let supervised = lease.is_some();
    command
        .arg("--root")
        .arg(&root)
        .args(["run", "--keep", "--bundle"])
        .arg(bundle_dir)
        .arg(id)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    opencoder_session::process::configure_owned_command(&mut command, supervised);
    let mut child = command.spawn().context("spawn runc")?;
    let mut supervisor = lease.map(|lease| lease.spawned(child.id())).transpose()?;
    let stdout = child.stdout.take().context("runc stdout")?;
    let stderr = child.stderr.take().context("runc stderr")?;
    let deadline = async {
        match timeout_secs {
            Some(secs) => tokio::time::sleep(Duration::from_secs(secs)).await,
            None => std::future::pending().await,
        }
    };
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(anyhow::anyhow!("runc step cancelled")),
        _ = deadline => Err(anyhow::anyhow!("runc step timeout")),
        result = async {
            let wait = async { Ok::<_, anyhow::Error>(child.wait().await?) };
            // Mirror each pipe through the step log when one was supplied.
            let stdout: Box<dyn tokio::io::AsyncRead + Unpin + Send> = match &output {
                Some(log) => Box::new(log.tee_reader(crate::step_log::Stream::Stdout, stdout)),
                None => Box::new(stdout),
            };
            let stderr: Box<dyn tokio::io::AsyncRead + Unpin + Send> = match &output {
                Some(log) => Box::new(log.tee_reader(crate::step_log::Stream::Stderr, stderr)),
                None => Box::new(stderr),
            };
            let (out, err, status) = tokio::try_join!(
                crate::sandbox::output_limit::read_logged(
                    stdout,
                    "runc stdout",
                    crate::sandbox::output_limit::STREAM_OUTPUT_LIMIT_BYTES,
                    None,
                ),
                crate::sandbox::output_limit::read_logged(
                    stderr,
                    "runc stderr",
                    crate::sandbox::output_limit::STREAM_OUTPUT_LIMIT_BYTES,
                    None,
                ),
                wait,
            )?;
            let mut text = String::from_utf8_lossy(&out).into_owned();
            if !err.is_empty() {
                text.push_str("\n-- stderr --\n");
                text.push_str(&String::from_utf8_lossy(&err));
            }
            Ok((status.code().unwrap_or(-1), text))
        } => result,
    };
    if let Some(supervisor) = &mut supervisor {
        supervisor.terminate();
    }
    if child.try_wait()?.is_none() {
        opencoder_session::process::wait_owned_child(&mut child, supervised)
            .await
            .context("reap runc owner")?;
    }
    let first_cleanup = delete_force(&root, id).await;
    // Recheck after launcher exit: cancellation can race container creation.
    // A successful second pass also resolves a transient first-pass race.
    if let Err(error) = delete_force(&root, id).await {
        anyhow::bail!(
            "runc cleanup failed: {error:#}; first cleanup: {first_cleanup:?}; step: {result:?}"
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cleanup_removes_interrupted_creation_without_container_metadata() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("interrupted-create");
        std::fs::create_dir(&state).unwrap();
        delete_force(root.path(), "interrupted-create")
            .await
            .unwrap();
        assert!(!state.exists());
        assert!(root.path().is_dir());
    }
}

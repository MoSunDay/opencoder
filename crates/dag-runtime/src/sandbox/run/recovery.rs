use anyhow::{ensure, Result};
use std::path::Path;

pub async fn cleanup(root: &Path, run_id: &str) -> Result<()> {
    ensure!(
        opencoder_dag::artifacts::validate_run_id(run_id),
        "invalid recovery run id"
    );
    super::mounts::real_dir(root)?;
    let expected = format!("dag-run-{run_id}");
    if root.join("container.json").exists() {
        let receipt: super::Receipt =
            serde_json::from_slice(&std::fs::read(root.join("container.json"))?)?;
        ensure!(receipt.id == expected, "DAG recovery ownership mismatch");
    }
    super::RunContainer {
        root: root.to_path_buf(),
        id: expected,
        cleaned: std::sync::atomic::AtomicBool::new(false),
    }
    .cleanup()
    .await
}

pub(super) fn cleanup_abandoned(root: &Path, id: &str) -> Result<()> {
    super::mounts::real_dir(root)?;
    let state = root.join("runc-state");
    if state.join(id).exists() {
        let mut child = std::process::Command::new("runc")
            .arg("--root")
            .arg(&state)
            .args(["delete", "--force", id])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Some(status) = child.try_wait()? {
                ensure!(status.success(), "delete abandoned DAG container failed");
                break;
            }
            if std::time::Instant::now() >= deadline {
                child.kill()?;
                child.wait()?;
                anyhow::bail!("delete abandoned DAG container timed out");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    super::mounts::unmount(&root.join("bundle/rootfs"))?;
    super::mounts::unmount(&root.join("workspace"))
}

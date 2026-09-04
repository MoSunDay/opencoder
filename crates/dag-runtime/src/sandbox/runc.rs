//! `runc` process driving for sandboxed python steps.
//!
//! Thin async wrappers around the `runc` binary (no shell, argv only).
//! Lifetime contract of [`run_step`]: after it returns — success, non-zero
//! exit, or timeout — a best-effort `runc delete --force` has been issued,
//! so container ids never leak across steps.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context as _, Result};
use tokio::io::AsyncReadExt as _;
use tokio::process::Command;
use tokio::time::timeout;

/// Is a usable `runc` on PATH? (`runc --version`; no shell involved.)
pub fn runc_available() -> bool {
    std::process::Command::new("runc")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Run one OCI bundle to completion.
///
/// Returns `(exit_code, output)` where `output` is the captured stdout with
/// stderr appended (`-- stderr --` separator) when non-empty. `exit_code` is
/// `-1` when the process died to a signal (e.g. after our timeout KILL).
///
/// On `timeout_secs` elapse: `runc kill <id> KILL`, wait briefly for the run
/// process to exit, `runc delete --force`, then `Err("runc step timeout")`.
async fn run_step_inner(bundle_dir: &Path, id: &str) -> Result<(i32, String)> {
    let mut child = Command::new("runc")
        .arg("run")
        .arg("--bundle")
        .arg(bundle_dir)
        .arg(id)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn runc")?;

    let mut stdout_pipe = child.stdout.take().context("runc stdout not piped")?;
    let mut stderr_pipe = child.stderr.take().context("runc stderr not piped")?;

    // Drain both pipes concurrently with the wait — otherwise a chatty step
    // deadlocks on a full pipe.
    let mut out = Vec::new();
    let mut err = Vec::new();
    let (out_res, err_res, status) = futures::join!(
        stdout_pipe.read_to_end(&mut out),
        stderr_pipe.read_to_end(&mut err),
        child.wait(),
    );
    out_res.context("read runc stdout")?;
    err_res.context("read runc stderr")?;
    let status = status.context("wait runc")?;

    let mut text = String::from_utf8_lossy(&out).into_owned();
    let err_text = String::from_utf8_lossy(&err).into_owned();
    if !err_text.trim().is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str("-- stderr --\n");
        text.push_str(&err_text);
    }
    Ok((status.code().unwrap_or(-1), text))
}

/// [`run_step_inner`] with the timeout + always-cleanup contract.
pub async fn run_step(
    bundle_dir: &Path,
    id: &str,
    timeout_secs: Option<u64>,
) -> Result<(i32, String)> {
    let result = match timeout_secs {
        None => run_step_inner(bundle_dir, id).await,
        Some(secs) => {
            match timeout(Duration::from_secs(secs), run_step_inner(bundle_dir, id)).await {
                Ok(inner) => inner,
                Err(_elapsed) => {
                    // Kill the container by id (the spawned runc process is
                    // detached once the inner future is dropped), then let the
                    // unconditional delete below reap it.
                    kill(id).await;
                    Err(anyhow::anyhow!("runc step timeout"))
                }
            }
        }
    };
    // Always reap the container, best-effort.
    delete_force(id).await;
    result
}

async fn kill(id: &str) {
    let _ = Command::new("runc")
        .args(["kill", id, "KILL"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}

async fn delete_force(id: &str) {
    let _ = Command::new("runc")
        .args(["delete", "--force", id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Candidate fixture roots checked by [`runc_step_smoke`] (skips when
    /// none exist — the must-pass tests are the pure OCI + VM ones).
    fn smoke_rootfs_candidates() -> Vec<PathBuf> {
        let mut roots = Vec::new();
        if let Ok(from_env) = std::env::var("DAG_TEST_ROOTFS") {
            roots.push(PathBuf::from(from_env));
        }
        roots.push(PathBuf::from("tests/fixtures/rootfs"));
        roots.push(PathBuf::from("/opt/opencoder/rootfs"));
        roots
    }

    /// End-to-end smoke through a real runc when both runc and a prepared
    /// rootfs fixture are present; otherwise reports and skips. Offline CI
    /// environments exercise the OCI + VM unit tests instead.
    #[tokio::test]
    async fn runc_step_smoke() {
        if !runc_available() {
            eprintln!("skipping: runc not installed");
            return;
        }
        // The shared rootfs must be a REAL directory named `rootfs` (runc
        // rejects symlinks) — its parent plays workflow root for the smoke.
        let Some(rootfs) = smoke_rootfs_candidates()
            .into_iter()
            .find(|p| p.is_dir() && p.file_name().is_some_and(|n| n == "rootfs"))
        else {
            eprintln!("skipping: no rootfs fixture (set DAG_TEST_ROOTFS to a directory named `rootfs` to enable)");
            return;
        };
        let workflow_root = rootfs.parent().expect("fixture has a parent").to_path_buf();
        std::fs::create_dir_all(&workflow_root).unwrap();

        let spec = crate::sandbox::oci::BundleSpec {
            run_root: workflow_root.join("run-1"),
            step_slug: "smoke".into(),
            code: "print('from runc')".into(),
            timeout_hint: Some(30),
        };
        let bundle = crate::sandbox::oci::write_bundle(&workflow_root.join("b"), &spec).unwrap();
        let (code, out) = run_step(&bundle, "dag-smoke-test", Some(30)).await.unwrap();
        assert_eq!(code, 0, "runc step output: {out}");
        assert!(out.contains("from runc"), "{out}");
    }
}

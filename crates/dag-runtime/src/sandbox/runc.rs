//! Bounded runc lifecycle: cancellation/timeout stops the container and
//! reaps the launcher before returning; cleanup failures remain visible.
use anyhow::{Context, Result};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

mod cleanup;
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

pub async fn run_step(
    bundle_dir: &Path,
    id: &str,
    timeout_secs: Option<u64>,
) -> Result<(i32, String)> {
    run_step_cancellable(bundle_dir, id, timeout_secs, CancellationToken::new()).await
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
    use std::path::PathBuf;
    use tokio::time::timeout;

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

    /// Candidate fixture roots checked by the explicitly invoked manual tests.
    /// Missing prerequisites fail the manual invocation.
    fn smoke_rootfs_candidates() -> Vec<PathBuf> {
        let mut roots = Vec::new();
        if let Ok(from_env) = std::env::var("DAG_TEST_ROOTFS") {
            roots.push(PathBuf::from(from_env));
        }
        roots.push(PathBuf::from("tests/fixtures/rootfs"));
        roots.push(PathBuf::from("/opt/opencoder/rootfs"));
        roots
    }

    fn stage_program(run_root: &Path, step: &str, source: &str) {
        let directory = run_root.join(step);
        std::fs::create_dir_all(&directory).unwrap();
        let source_path = directory.join("program.c");
        std::fs::write(&source_path, source).unwrap();
        let result = std::process::Command::new("cc")
            .args(["-O2", "-static"])
            .arg(&source_path)
            .arg("-o")
            .arg(directory.join("program"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    const HELLO_C: &str = "#include <stdio.h>\nint main(void) { puts(\"from runc\"); return 0; }";
    const SPIN_C: &str = "#include <unistd.h>\nint main(void) { for (;;) pause(); }";
    const OVERFLOW_C: &str = "#include <stdio.h>\n#include <string.h>\nint main(void) { char bytes[65536]; memset(bytes, 'x', sizeof(bytes)); for (int chunk=0; chunk<150; chunk++) fwrite(bytes, sizeof(bytes), 1, stdout); return 0; }";

    /// End-to-end smoke through a real runc when both runc and a prepared
    /// rootfs fixture are present. Explicit manual invocation fails if either
    /// prerequisite is absent; ordinary CI reports this test as ignored.
    #[tokio::test]
    #[ignore = "manual: requires runc and prepared native rootfs"]
    async fn runc_step_smoke() {
        assert!(runc_available(), "runc not installed");
        let rootfs = smoke_rootfs_candidates()
            .into_iter()
            .find(|p| p.is_dir() && p.file_name().is_some_and(|n| n == "rootfs"))
            .expect("set DAG_TEST_ROOTFS to a prepared directory named rootfs");
        let workflow_root = rootfs.parent().expect("fixture has a parent").to_path_buf();
        std::fs::create_dir_all(&workflow_root).unwrap();

        let spec = crate::sandbox::oci::BundleSpec {
            run_root: workflow_root.join("run-1"),
            step_slug: "smoke".into(),
            command: vec!["/workspace/context/smoke/program".into()],
            env: Vec::new(),
            timeout_hint: Some(30),
            knowledge: None,
            agents: None,
        };
        stage_program(&spec.run_root, "smoke", HELLO_C);
        let bundle = crate::sandbox::oci::write_bundle(&workflow_root.join("b"), &spec).unwrap();
        // A container combines independently valid run and step IDs.
        let id = format!("{}-{}", "r".repeat(64), "s".repeat(64));
        let (code, out) = run_step(&bundle, &id, Some(30)).await.unwrap();
        assert_eq!(code, 0, "runc step output: {out}");
        assert!(out.contains("from runc"), "{out}");
    }

    #[tokio::test]
    #[ignore = "manual: requires runc and prepared native rootfs"]
    async fn cancellation_and_timeout_remove_running_containers() {
        assert!(runc_available());
        let rootfs = smoke_rootfs_candidates()
            .into_iter()
            .find(|p| p.is_dir())
            .expect("set DAG_TEST_ROOTFS");
        let workflow = rootfs.parent().unwrap();
        for timed_out in [false, true] {
            let id = format!("dag-stop-{}", ulid::Ulid::new());
            let spec = crate::sandbox::oci::BundleSpec {
                run_root: workflow.join(&id),
                step_slug: "loop".into(),
                command: vec!["/workspace/context/loop/program".into()],
                env: Vec::new(),
                timeout_hint: timed_out.then_some(5),
                knowledge: None,
                agents: None,
            };
            stage_program(&spec.run_root, "loop", SPIN_C);
            let bundle =
                crate::sandbox::oci::write_bundle(&workflow.join(format!("bundle-{id}")), &spec)
                    .unwrap();
            let cancel = CancellationToken::new();
            let child_cancel = cancel.clone();
            let child_bundle = bundle.clone();
            let child_id = id.clone();
            let execution = tokio::spawn(async move {
                run_step_cancellable(
                    &child_bundle,
                    &child_id,
                    timed_out.then_some(5),
                    child_cancel,
                )
                .await
            });
            // The native program writes nothing observable, so wait for runc's
            // private container state directory instead.
            let state_dir = bundle.join("runc-state").join(&id);
            timeout(Duration::from_secs(15), async {
                while !state_dir.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("container started");
            if !timed_out {
                cancel.cancel();
            }
            let error = timeout(Duration::from_secs(15), execution)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(if timed_out { "timeout" } else { "cancelled" }),
                "{error:#}"
            );
            assert!(!bundle.join("runc-state").join(&id).exists());
        }
    }

    #[tokio::test]
    #[ignore = "manual: requires runc and prepared native rootfs"]
    async fn stdout_overflow_fails_and_removes_container() {
        assert!(runc_available());
        let rootfs = smoke_rootfs_candidates()
            .into_iter()
            .find(|path| path.is_dir())
            .expect("set DAG_TEST_ROOTFS");
        let workflow = rootfs.parent().unwrap();
        let id = format!("dag-overflow-{}", ulid::Ulid::new());
        let bundle_path = workflow.join(format!("bundle-{id}"));
        let spec = crate::sandbox::oci::BundleSpec {
            run_root: workflow.join(&id),
            step_slug: "overflow".into(),
            command: vec!["/workspace/context/overflow/program".into()],
            env: Vec::new(),
            timeout_hint: Some(30),
            knowledge: None,
            agents: None,
        };
        stage_program(&spec.run_root, "overflow", OVERFLOW_C);
        let bundle = crate::sandbox::oci::write_bundle(&bundle_path, &spec).unwrap();
        let error = run_step(&bundle, &id, Some(30)).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("output_limit_exceeded: runc stdout exceeds 8388608 bytes"),
            "{error:#}"
        );
        assert!(!bundle.join("runc-state").join(&id).exists());
        let _ = std::fs::remove_dir_all(&bundle_path);
        let _ = std::fs::remove_dir_all(&spec.run_root);
    }
}

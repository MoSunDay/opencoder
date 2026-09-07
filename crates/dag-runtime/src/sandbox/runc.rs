//! Bounded runc lifecycle: cancellation/timeout stops the container and
//! reaps the launcher before returning; cleanup failures remain visible.
use anyhow::{Context, Result};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{process::Command, time::timeout};
use tokio_util::sync::CancellationToken;

mod recovery;
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
            let (out, err, status) = tokio::try_join!(
                crate::sandbox::output_limit::read_bounded(
                    stdout,
                    "runc stdout",
                    crate::sandbox::output_limit::STREAM_OUTPUT_LIMIT_BYTES,
                ),
                crate::sandbox::output_limit::read_bounded(
                    stderr,
                    "runc stderr",
                    crate::sandbox::output_limit::STREAM_OUTPUT_LIMIT_BYTES,
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

pub(super) async fn delete_force(root: &Path, id: &str) -> Result<()> {
    if !root.join(id).exists() {
        return Ok(());
    }
    let mut child = Command::new("runc")
        .arg("--root")
        .arg(root)
        .args(["delete", "--force", id])
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn runc cleanup")?;
    let status = match timeout(Duration::from_secs(5), child.wait()).await {
        Ok(status) => status?,
        Err(_) => {
            child.kill().await.context("reap timed-out runc cleanup")?;
            anyhow::bail!("runc delete exceeded 5s");
        }
    };
    anyhow::ensure!(
        status.success() || !root.join(id).exists(),
        "runc delete failed: {status}"
    );
    anyhow::ensure!(
        !root.join(id).exists(),
        "runc container state remains after cleanup"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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

    /// Stage a wat module as `<run_root>/<step>/module.wasm` — the
    /// executor-equivalent placement the container command references.
    fn stage_module(run_root: &Path, step: &str, wat: &str) {
        let dir = run_root.join(step);
        std::fs::create_dir_all(&dir).unwrap();
        let wasm = wat::parse_str(wat).unwrap();
        std::fs::write(dir.join("module.wasm"), wasm).unwrap();
    }

    const HELLO_WAT: &str = r#"
        (module
            (import "wasi_snapshot_preview1" "fd_write"
                (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "from runc\n")
            (func (export "_start")
                (i32.store (i32.const 16) (i32.const 0))
                (i32.store (i32.const 20) (i32.const 10))
                (drop
                    (call $fd_write
                        (i32.const 1) (i32.const 16) (i32.const 1) (i32.const 24)))))
    "#;

    const SPIN_WAT: &str = r#"
        (module
            (func (export "_start") (loop $l (br $l))))
    "#;

    /// Fills the first 64 KiB with `x`, then streams it to stdout until the
    /// 8 MiB runc output cap is blown (~150 x 64 KiB writes).
    const OVERFLOW_WAT: &str = r#"
        (module
            (import "wasi_snapshot_preview1" "fd_write"
                (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 2)
            (func (export "_start")
                (local $i i32)
                (loop $fill
                    (i32.store8 (local.get $i) (i32.const 120))
                    (local.set $i (i32.add (local.get $i) (i32.const 1)))
                    (br_if $fill (i32.lt_u (local.get $i) (i32.const 65536))))
                (i32.store (i32.const 65536) (i32.const 0))
                (i32.store (i32.const 65540) (i32.const 65536))
                (local.set $i (i32.const 0))
                (loop $write
                    (drop
                        (call $fd_write
                            (i32.const 1) (i32.const 65536) (i32.const 1) (i32.const 65544)))
                    (local.set $i (i32.add (local.get $i) (i32.const 1)))
                    (br_if $write (i32.lt_u (local.get $i) (i32.const 150))))))
    "#;

    /// End-to-end smoke through a real runc when both runc and a prepared
    /// rootfs fixture are present. Explicit manual invocation fails if either
    /// prerequisite is absent; ordinary CI reports this test as ignored.
    #[tokio::test]
    #[ignore = "manual: requires runc and prepared wasm rootfs"]
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
            command: vec!["smoke/module.wasm".into()],
            env: Vec::new(),
            timeout_hint: Some(30),
        };
        stage_module(&spec.run_root, "smoke", HELLO_WAT);
        let bundle = crate::sandbox::oci::write_bundle(&workflow_root.join("b"), &spec).unwrap();
        // A container combines independently valid run and step IDs.
        let id = format!("{}-{}", "r".repeat(64), "s".repeat(64));
        let (code, out) = run_step(&bundle, &id, Some(30)).await.unwrap();
        assert_eq!(code, 0, "runc step output: {out}");
        assert!(out.contains("from runc"), "{out}");
    }

    #[tokio::test]
    #[ignore = "manual: requires runc and prepared wasm rootfs"]
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
                command: vec!["loop/module.wasm".into()],
                env: Vec::new(),
                timeout_hint: timed_out.then_some(5),
            };
            stage_module(&spec.run_root, "loop", SPIN_WAT);
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
            // The spin module writes nothing observable, so wait for runc's
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
    #[ignore = "manual: requires runc and prepared wasm rootfs"]
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
            command: vec!["overflow/module.wasm".into()],
            env: Vec::new(),
            timeout_hint: Some(30),
        };
        stage_module(&spec.run_root, "overflow", OVERFLOW_WAT);
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

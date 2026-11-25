mod config;
mod mounts;
mod output;
mod process;
mod recovery;

use anyhow::{ensure, Context, Result};
use super::clone3::Backend;
use opencoder_dag::DagClaimedRun;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::process::Command;

pub(crate) use mounts::real_dir as validate_directory;
pub use output::ProcessFailure;
pub use process::{execute, StepProcess};
pub use recovery::cleanup as cleanup_run;

/// Per-run record under `container.json`; `execute` reads it to reach the
/// backend without the node configuration.
#[derive(Serialize, Deserialize)]
pub(crate) struct Receipt {
    pub id: String,
    pub agent_uid: u32,
    /// Backend owning the run; records written before clone3 existed are runc.
    #[serde(default)]
    pub backend: Backend,
    /// Host path of the rootfs `dag-runner`, the clone3 launcher and relay.
    #[serde(default)]
    pub runner: Option<PathBuf>,
}

pub struct RunContainer {
    root: PathBuf,
    id: String,
    cleaned: AtomicBool,
    /// Live clone3 sandbox; `None` for runc, whose state lives in `runc-state`.
    launcher: Option<super::clone3::launch::Launcher>,
}

impl RunContainer {
    pub async fn start(
        root: &Path,
        config: &opencoder_core::Config,
        run: &DagClaimedRun,
    ) -> Result<Self> {
        let backend = super::clone3::select(config)?;
        preflight(config)?;
        ensure!(
            opencoder_dag::artifacts::validate_run_id(&run.run_id),
            "invalid DAG run id"
        );
        std::fs::create_dir_all(root)?;
        mounts::real_dir(root)?;
        let id = format!("dag-run-{}", run.run_id);
        let mut owner = Self {
            root: root.to_path_buf(),
            id,
            cleaned: AtomicBool::new(false),
            launcher: None,
        };
        owner.cleanup().await?;
        owner.cleaned.store(false, Ordering::Release);
        if let Err(error) = owner.prepare(config, run, backend) {
            owner
                .cleanup()
                .await
                .context("cleanup failed DAG preparation")?;
            return Err(error);
        }
        let diagnostic = root.join("private/start.log");
        let stderr = std::fs::File::create(&diagnostic)?;
        let started = match backend {
            Backend::Runc => owner.start_runc(stderr).await.map(|_| None),
            Backend::Clone3 => owner.start_clone3(config, stderr).await.map(Some),
        };
        let launcher = match started {
            Ok(launcher) => launcher,
            Err(error) => {
                owner.cleanup().await?;
                return Err(error);
            }
        };
        owner.launcher = launcher;
        if let Err(error) = owner.startup_check(run).await {
            owner.cleanup().await?;
            return Err(error);
        }
        Ok(owner)
    }

    /// runc keeps the container's state under `runc-state`.
    async fn start_runc(&self, stderr: std::fs::File) -> Result<()> {
        let status = Command::new("runc")
            .arg("--root")
            .arg(self.root.join("runc-state"))
            .args(["run", "--no-pivot", "--detach", "--bundle"])
            .arg(self.root.join("bundle"))
            .arg(&self.id)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr)
            .status()
            .await
            .context("start DAG container")?;
        if status.success() {
            return Ok(());
        }
        let diagnostic = self.root.join("private/start.log");
        let bytes = super::output_limit::read_file_bounded(&diagnostic, "DAG startup", 64 * 1024)?;
        anyhow::bail!("DAG container failed: {}", String::from_utf8_lossy(&bytes))
    }

    /// The clone3 sandbox is the rootfs launcher process, whose stdin is
    /// this owner's lease over the sandbox's lifetime.
    async fn start_clone3(
        &self,
        config: &opencoder_core::Config,
        log: std::fs::File,
    ) -> Result<super::clone3::launch::Launcher> {
        let rootfs = config
            .dag
            .rootfs_dir
            .as_deref()
            .context("DAG rootfs_dir is required")?;
        super::clone3::launch::spawn(&self.root, &rootfs.join("usr/bin/dag-runner"), log).await
    }

    async fn startup_check(&self, run: &DagClaimedRun) -> Result<()> {
        let verified = execute(
            &self.root,
            StepProcess {
                key: "startup-check".into(),
                argv: vec![
                    "/usr/bin/dag-runner".into(),
                    "check".into(),
                    run.run_id.clone(),
                ],
                env: vec![],
                cwd: "/workspace".into(),
                timeout_secs: Some(10),
            },
            tokio_util::sync::CancellationToken::new(),
            None,
        )
        .await;
        match verified {
            Ok((0, _)) => Ok(()),
            failure => Err(anyhow::anyhow!(
                "DAG container execution preflight failed: {failure:?}"
            )),
        }
    }

    fn prepare(
        &self,
        config: &opencoder_core::Config,
        run: &DagClaimedRun,
        backend: Backend,
    ) -> Result<()> {
        let source = config
            .dag
            .workspace_dir
            .as_deref()
            .context("DAG workspace_dir is required")?;
        let rootfs = config::rootfs_source(config)?;
        let uid = unsafe { libc::geteuid() };
        ensure!(
            uid != 65532,
            "node credential user cannot be the binary execution user"
        );
        for directory in ["bundle", "private", "private/exec"] {
            std::fs::create_dir_all(self.root.join(directory))?;
        }
        // Only the OCI backend keeps container state under the run root.
        if backend == Backend::Runc {
            std::fs::create_dir_all(self.root.join("runc-state"))?;
        }
        mounts::permissions(&self.root.join("private"), uid, 0o700)?;
        opencoder_core::atomic_write(&self.root.join("private/identity"), run.run_id.as_bytes())?;
        mounts::overlay(
            source,
            &self.root.join("upper"),
            &self.root.join("work"),
            &self.root.join("workspace"),
        )?;
        mounts::permissions(&self.root.join("workspace"), uid, 0o2770)?;
        mounts::overlay(
            &rootfs,
            &self.root.join("rootfs-upper"),
            &self.root.join("rootfs-work"),
            &self.root.join("bundle/rootfs"),
        )?;
        let value = config::render(&self.root, config, run, uid)?;
        // The node materialises the configured hostname inside the run's own
        // rootfs overlay for both backends: runc and the native launcher set
        // the same kernel value, and runc alone would leave no file behind.
        if let Some(host) = value["hostname"].as_str().filter(|host| !host.is_empty()) {
            let rootfs = self.root.join("bundle/rootfs");
            if rootfs.join("etc").is_dir() {
                std::fs::write(rootfs.join("etc/hostname"), format!("{host}\n"))
                    .context("write sandbox hostname file")?;
            }
        }
        opencoder_core::atomic_write(
            &self.root.join("bundle/config.json"),
            &serde_json::to_vec_pretty(&value)?,
        )?;
        opencoder_core::atomic_write(
            &self.root.join("container.json"),
            &serde_json::to_vec(&Receipt {
                id: self.id.clone(),
                agent_uid: uid,
                backend,
                runner: (backend == Backend::Clone3)
                    .then(|| rootfs.join("usr/bin/dag-runner")),
            })?,
        )?;
        Ok(())
    }

    pub async fn cleanup(&self) -> Result<()> {
        match &self.launcher {
            Some(launcher) => launcher.stop().await?,
            None => super::clone3::launch::stop_abandoned(&self.root).await?,
        }
        let state = self.root.join("runc-state");
        if state.join(&self.id).exists() {
            super::runc::delete_force(&state, &self.id).await?;
        }
        let root = self.root.clone();
        let unmount = tokio::task::spawn_blocking(move || {
            let result = mounts::unmount(&root.join("bundle/rootfs"))
                .and_then(|_| mounts::unmount(&root.join("workspace")));
            if let Err(error) = &result {
                tracing::error!(%error, path = %root.display(), "DAG mount cleanup failed");
            }
            result
        });
        finish_unmount(&self.cleaned, unmount).await
    }
}

async fn finish_unmount(
    cleaned: &AtomicBool,
    unmount: tokio::task::JoinHandle<Result<()>>,
) -> Result<()> {
    // A timeout cannot cancel the kernel's unmount. Keep ownership and wait
    // for the actual result, including slow NFS/OverlayFS writeback, before
    // reporting a terminal run or releasing its capacity.
    unmount.await.context("DAG cleanup task failed")??;
    cleaned.store(true, Ordering::Release);
    Ok(())
}

impl Drop for RunContainer {
    fn drop(&mut self) {
        if !self.cleaned.load(Ordering::Acquire) {
            let root = self.root.clone();
            let id = self.id.clone();
            let cleanup = move || {
                if let Err(error) = recovery::cleanup_abandoned(&root, &id) {
                    tracing::error!(%error, %id, "DAG container cleanup failed after owner dropped");
                }
            };
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn_blocking(cleanup);
            } else {
                std::thread::spawn(cleanup);
            }
        }
    }
}

pub fn preflight(config: &opencoder_core::Config) -> Result<()> {
    if super::clone3::select(config)? == Backend::Runc {
        ensure!(
            super::runc::runc_available(),
            "runc is required for every DAG run"
        );
    }
    config::rootfs_source(config)?;
    let source = config
        .dag
        .workspace_dir
        .as_deref()
        .context("DAG workspace_dir is required")?;
    mounts::real_dir(source)?;
    crate::nfs::read_only_mount(source)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, time::Duration};

    #[tokio::test(start_paused = true)]
    async fn cleanup_keeps_ownership_until_slow_unmount_finishes() {
        let cleaned = Arc::new(AtomicBool::new(false));
        let (release, pending) = tokio::sync::oneshot::channel();
        let unmount = tokio::spawn(async move {
            pending.await?;
            Ok(())
        });
        let marker = cleaned.clone();
        let cleanup = tokio::spawn(async move { finish_unmount(&marker, unmount).await });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(21)).await;
        assert!(!cleanup.is_finished());
        assert!(!cleaned.load(Ordering::Acquire));
        release.send(()).unwrap();
        cleanup.await.unwrap().unwrap();
        assert!(cleaned.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn failed_unmount_preserves_ownership_for_cleanup_retry() {
        let cleaned = AtomicBool::new(false);
        let unmount = tokio::spawn(async { anyhow::bail!("fixture mount still busy") });
        let error = finish_unmount(&cleaned, unmount).await.unwrap_err();
        assert!(error.to_string().contains("fixture mount still busy"));
        assert!(!cleaned.load(Ordering::Acquire));
    }
}

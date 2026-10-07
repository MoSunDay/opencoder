use super::mounts;
#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
use anyhow::{ensure, Context, Result};
use opencoder_dag::{DagClaimedRun, StepKind};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub(super) fn capabilities() -> Value {
    let filesystem = ["CAP_CHOWN", "CAP_DAC_OVERRIDE", "CAP_FOWNER"];
    json!({"bounding":filesystem,"effective":filesystem,"inheritable":[],"permitted":filesystem,"ambient":[]})
}

fn bind(
    rootfs: &Path,
    mounts: &mut Vec<Value>,
    source: &Path,
    destination: &str,
    writable: bool,
) -> Result<()> {
    mounts::real_dir(source)?;
    let target = super::super::codex::guest_path(rootfs, destination)?;
    std::fs::create_dir_all(&target)?;
    mounts.push(
        json!({"type":"bind","source":source,"destination":destination,
        "options":["rbind",if writable {"rw"} else {"ro"},"nosuid","nodev"]}),
    );
    Ok(())
}

pub(super) fn render(
    root: &Path,
    config: &opencoder_core::Config,
    run: &DagClaimedRun,
    agent_uid: u32,
) -> Result<Value> {
    let rootfs = root.join("bundle/rootfs");
    let mut mounts = vec![
        json!({"destination":"/proc","type":"proc","source":"proc","options":["nosuid","nodev","noexec"]}),
        json!({"destination":"/tmp","type":"tmpfs","source":"tmpfs","options":["rw","nosuid","nodev","mode=1777","size=256m"]}),
        json!({"destination":"/dev","type":"tmpfs","source":"tmpfs","options":["rw","nosuid","mode=755","size=16m"]}),
    ];
    bind(
        &rootfs,
        &mut mounts,
        &root.join("workspace"),
        "/workspace",
        true,
    )?;
    bind(
        &rootfs,
        &mut mounts,
        &root.join("private"),
        "/run/opencoder-private",
        false,
    )?;
    mounts::permissions(&rootfs.join("run/opencoder-private"), agent_uid, 0o700)?;
    if let Some(source) = config.dag.knowledge_root.as_deref() {
        bind(
            &rootfs,
            &mut mounts,
            source,
            crate::exec::native::io::KNOWLEDGE_MOUNT,
            false,
        )?;
    }
    if let Some(source) = config.agent.agents_dir.as_deref() {
        bind(
            &rootfs,
            &mut mounts,
            source,
            crate::exec::native::io::AGENTS_MOUNT,
            false,
        )?;
    }
    if let Some(source) = config.dag.execution_private_root.as_deref() {
        bind(
            &rootfs,
            &mut mounts,
            source,
            opencoder_core::fleet::private_files::GUEST_ROOT,
            false,
        )?;
    }
    for step in &run.spec.steps {
        crate::exec::native::files::create_directory(
            &root.join("workspace"),
            Path::new(&step.name),
        )?;
        crate::exec::native::files::create_directory(
            &root.join("workspace"),
            &Path::new(&step.name).join("meta"),
        )?;
        let meta = root.join(&step.name).join("meta");
        std::fs::create_dir_all(&meta)?;
        bind(
            &rootfs,
            &mut mounts,
            &meta,
            &format!("/workspace/{}/meta", step.name),
            false,
        )?;
        if let StepKind::Agent { agent, .. } = step.kind.executable() {
            if let Some(launch) =
                super::super::codex::resolve(config, agent.as_deref().unwrap_or("act"), &rootfs)?
            {
                use std::os::unix::fs::MetadataExt;
                ensure!(
                    std::fs::metadata(&launch.home)?.uid() == agent_uid,
                    "Codex credentials must belong to the executing node user"
                );
                let private = root.join("private/codex").join(&step.name);
                std::fs::create_dir_all(&private)?;
                mounts::permissions(&private, agent_uid, 0o700)?;
                opencoder_core::atomic_write(
                    &private.join("launch.json"),
                    &serde_json::to_vec(&launch.runtime)?,
                )?;
                let guest = launch.home.to_str().context("Codex home must be UTF-8")?;
                ensure!(
                    !mounts.iter().any(|mount| {
                        let destination = Path::new(mount["destination"].as_str().unwrap_or("/"));
                        (destination == launch.home && mount["source"] != json!(launch.home))
                            || (destination != launch.home && destination.starts_with(&launch.home))
                    }),
                    "Codex home must not shadow a runtime mount"
                );
                if !mounts.iter().any(|mount| mount["destination"] == guest) {
                    bind(&rootfs, &mut mounts, &launch.home, guest, true)?;
                }
            }
        }
    }
    Ok(json!({"ociVersion":"1.0.0","hostname":"dag-run",
        "annotations":{"org.opencoder.dag.run":run.run_id,"org.opencoder.dag.definition":run.dag_id},
        "process":{"terminal":false,"user":{"uid":agent_uid,"gid":65532},"args":["/usr/bin/dag-runner","init"],
            "env":["PATH=/usr/bin:/bin"],"cwd":"/workspace","noNewPrivileges":true,
            "capabilities":capabilities()},
        "root":{"path":"rootfs","readonly":true},"mounts":mounts,
        "linux":{"namespaces":[{"type":"pid"},{"type":"ipc"},{"type":"uts"},{"type":"mount"}],
            "maskedPaths":["/proc/kcore","/proc/keys","/proc/timer_list"],
            "readonlyPaths":["/proc/sys","/proc/sysrq-trigger"]}}))
}

pub(super) fn rootfs_source(config: &opencoder_core::Config) -> Result<PathBuf> {
    let path = config
        .dag
        .rootfs_dir
        .as_ref()
        .context("DAG rootfs_dir is required")?;
    mounts::real_dir(path)?;
    let expected = serde_json::to_value(opencoder_core::version::build_info())?;
    for binary in ["dag-runner", "agent-step-runner"] {
        let runner = path.join("usr/bin").join(binary);
        let metadata = std::fs::symlink_metadata(&runner)
            .with_context(|| format!("rootfs is missing {binary}"))?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "rootfs {binary} must be a regular file"
        );
        let info = std::process::Command::new(&runner)
            .arg("--build-info")
            .output()
            .with_context(|| format!("read rootfs {binary} build metadata"))?;
        ensure!(
            info.status.success() && info.stdout.len() <= 8192,
            "rootfs {binary} build metadata unavailable"
        );
        let actual: Value = serde_json::from_slice(&info.stdout)
            .with_context(|| format!("invalid rootfs {binary} build metadata"))?;
        ensure!(
            actual == expected,
            "rootfs {binary} version does not match this execution node"
        );
    }
    Ok(path.clone())
}

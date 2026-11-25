use super::mounts;
#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
use crate::sandbox::host;
use anyhow::{ensure, Context, Result};
use opencoder_dag::{DagClaimedRun, StepKind};
use serde_json::{json, Value};
use std::path::Path;

pub(super) fn capabilities() -> Value {
    let filesystem = ["CAP_CHOWN", "CAP_DAC_OVERRIDE", "CAP_FOWNER"];
    json!({"bounding":filesystem,"effective":filesystem,"inheritable":[],"permitted":filesystem,"ambient":[]})
}

/// A shallow bind of one host path: only its declared path, without inheriting
/// writable submounts. The runtime creates missing mountpoints inside the
/// sandbox; nothing is mounted on the host.
fn bind(entries: &mut Vec<Value>, source: &Path, destination: &str, writable: bool) -> Result<()> {
    mounts::real_dir(source)?;
    entries.push(
        json!({"type":"bind","source":source,"destination":destination,
        "options":["bind",if writable {"rw"} else {"ro"},"nosuid","nodev"]}),
    );
    Ok(())
}

// Freeze the visible knowledge submounts as individual read-only binds. A plain
// bind hides them; rbind + ro only protects the top mount on older Linux/runc.
fn bind_knowledge(entries: &mut Vec<Value>, mountinfo: &str, source: &Path) -> Result<()> {
    let destination = crate::exec::native::io::KNOWLEDGE_MOUNT;
    bind(entries, source, destination, false)?;
    let source = source.canonicalize()?;
    for nested in mounts::nested_mounts(mountinfo, &source)? {
        let guest = Path::new(destination).join(nested.strip_prefix(&source)?);
        let guest = guest
            .to_str()
            .context("knowledge mount path must be UTF-8")?;
        let metadata = std::fs::symlink_metadata(&nested)?;
        if metadata.is_dir() {
            bind(entries, &nested, guest, false)?;
        } else {
            // The host placeholder of a mounted file is the sandbox mountpoint.
            ensure!(
                metadata.is_file(),
                "knowledge submount must be a directory or regular file"
            );
            mounts::real_dir(
                nested
                    .parent()
                    .context("knowledge submount has no parent")?,
            )?;
            entries.push(json!({"type":"bind","source":nested,"destination":guest,
                "options":["bind","ro","nosuid","nodev"]}));
        }
    }
    Ok(())
}

/// The OCI bundle of one run. `host_root` maps the host directories every
/// sandbox binds read-only; `root` owns the run's workspace and private
/// directories. Mountpoints are not created here: the root is a tmpfs.
pub(super) fn render(
    root: &Path,
    host_root: &Path,
    config: &opencoder_core::Config,
    run: &DagClaimedRun,
    agent_uid: u32,
) -> Result<Value> {
    let mountinfo = std::fs::read_to_string("/proc/self/mountinfo")?;
    let mut mounts = host::mounts(&mountinfo, host_root, &host::runners_dir(config)?)?;
    mounts.extend([
        json!({"destination":"/proc","type":"proc","source":"proc","options":["nosuid","nodev","noexec"]}),
        json!({"destination":"/tmp","type":"tmpfs","source":"tmpfs","options":["rw","nosuid","nodev","mode=1777","size=256m"]}),
        json!({"destination":"/dev","type":"tmpfs","source":"tmpfs","options":["rw","nosuid","mode=755","size=16m"]}),
        json!({"destination":"/dev/pts","type":"devpts","source":"devpts",
            "options":["rw","nosuid","noexec","newinstance","ptmxmode=0666","mode=0620","gid=5"]}),
    ]);
    bind(&mut mounts, &root.join("workspace"), "/workspace", true)?;
    bind(
        &mut mounts,
        &root.join("private"),
        "/run/opencoder-private",
        false,
    )?;
    if let Some(source) = config.dag.knowledge_root.as_deref() {
        bind_knowledge(&mut mounts, &mountinfo, source)?;
    }
    if let Some(source) = config.agent.agents_dir.as_deref() {
        bind(
            &mut mounts,
            source,
            crate::exec::native::io::AGENTS_MOUNT,
            false,
        )?;
    }
    if let Some(source) = config.dag.execution_private_root.as_deref() {
        bind(
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
            &mut mounts,
            &meta,
            &format!("/workspace/{}/meta", step.name),
            false,
        )?;
        if let StepKind::Agent { agent, .. } = step.kind.executable() {
            if let Some(launch) =
                super::super::codex::resolve(config, agent.as_deref().unwrap_or("act"), host_root)?
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
                    bind(&mut mounts, &launch.home, guest, true)?;
                }
                if let Some(startup) = &launch.startup {
                    startup.prepare(root, &step.name, &mut mounts)?;
                }
            }
        }
    }
    let runner = format!("{}/{}", host::RUNNERS_MOUNT, host::RUNNER);
    Ok(json!({"ociVersion":"1.0.0","hostname":"dag-run",
        "annotations":{"org.opencoder.dag.run":run.run_id,"org.opencoder.dag.definition":run.dag_id},
        "process":{"terminal":false,"user":{"uid":agent_uid,"gid":65532},"args":[runner,"init"],
            "env":[format!("PATH={}", host::GUEST_PATH)],"cwd":"/workspace","noNewPrivileges":true,
            "capabilities":capabilities()},
        // `root` is the empty bundle anchor; the first mount is the tmpfs `/`
        // and `readonly` seals it after every entry is mounted.
        "root":{"path":"root","readonly":true},"mounts":mounts,
        "linux":{"namespaces":[{"type":"pid"},{"type":"ipc"},{"type":"uts"},{"type":"mount"}],
            "maskedPaths":["/proc/kcore","/proc/keys","/proc/timer_list"],
            "readonlyPaths":["/proc/sys","/proc/sysrq-trigger"]}}))
}

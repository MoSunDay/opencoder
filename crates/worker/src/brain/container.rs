use crate::Worker;
use anyhow::{ensure, Context, Result};
use opencoder_core::{brain::layered::*, Config};
use serde_json::json;
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

pub fn cli_path() -> Result<PathBuf> {
    let path = std::env::current_exe()?
        .parent()
        .context("agent executable has no parent")?
        .join("opencoder-cli");
    ensure!(
        path.is_file(),
        "Brain requires opencoder-cli beside opencoder-agent: {}",
        path.display()
    );
    Ok(path)
}

pub async fn layered(
    worker: &Worker,
    config: &Config,
    context: &LayeredContext,
    cancel: CancellationToken,
) -> Result<LayeredDecision> {
    if let Some(client) = &worker.inner.client {
        return tokio::select! {
            _ = cancel.cancelled() => anyhow::bail!("brain activation cancelled"),
            result = opencoder_brain::layered::activate(context, client.as_ref(), config.model_id()) => result,
        };
    }
    activate_json(
        worker,
        config,
        &context.run_id,
        context.generation,
        context,
        cancel,
    )
    .await
}

async fn activate_json<T: serde::de::DeserializeOwned>(
    worker: &Worker,
    config: &Config,
    run_id: &str,
    generation: u64,
    context: &impl serde::Serialize,
    cancel: CancellationToken,
) -> Result<T> {
    ensure!(
        opencoder_dag_runtime::sandbox::runc::runc_available(),
        "Brain requires runc"
    );
    let cli = cli_path()?;
    let root = worker
        .inner
        .layout
        .execution_dir(opencoder_core::fleet::ExecutionKind::Brain, run_id)?;
    let activation = root.join("activations").join(generation.to_string());
    let bundle = worker
        .inner
        .layout
        .kind_root(opencoder_core::fleet::ExecutionKind::Brain)
        .join("bundles")
        .join(run_id)
        .join(generation.to_string());
    std::fs::create_dir_all(&activation)?;
    private_json(&activation.join("context.json"), context)?;
    private_json(&activation.join("config.json"), config)?;
    let exclusions = opencoder_core::net::proxy_bypass_environment(|key| std::env::var(key).ok());
    write_bundle(&bundle, &activation, &cli, &exclusions)?;
    let id = format!("{run_id}-a{generation}");
    let (code, output) =
        opencoder_dag_runtime::sandbox::runc::run_step_cancellable(&bundle, &id, Some(300), cancel)
            .await?;
    ensure!(code == 0, "Brain container exited {code}: {output}");
    let bytes = std::fs::read(activation.join("decision.json"))
        .context("activation omitted decision receipt")?;
    ensure!(
        bytes.len() <= 1024 * 1024,
        "activation decision exceeds 1 MiB"
    );
    serde_json::from_slice(&bytes).context("invalid layered decision")
}

fn private_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    opencoder_core::atomic_write_json(path, &serde_json::to_value(value)?)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn write_bundle(
    bundle: &Path,
    activation: &Path,
    cli: &Path,
    exclusions: &[(String, String)],
) -> Result<()> {
    let rootfs = bundle.join("rootfs");
    for dir in [
        "usr",
        "lib",
        "lib64",
        "bin",
        "etc",
        "proc",
        "dev",
        "tmp",
        "workspace",
        "runtime",
    ] {
        std::fs::create_dir_all(rootfs.join(dir))?;
    }
    let mut mounts = vec![
        json!({"destination":"/proc","type":"proc","source":"proc"}),
        json!({"destination":"/tmp","type":"tmpfs","source":"tmpfs","options":["nosuid","nodev","size=64m"]}),
        json!({"destination":"/workspace","type":"bind","source":activation,"options":["rbind","rw"]}),
    ];
    for host in [
        "/usr",
        "/lib",
        "/lib64",
        "/bin",
        "/etc/ssl",
        "/etc/resolv.conf",
        "/etc/hosts",
        "/dev/null",
        "/dev/urandom",
    ] {
        let path = Path::new(host);
        if !path.exists() {
            continue;
        }
        let target = rootfs.join(host.trim_start_matches('/'));
        if path.is_dir() {
            std::fs::create_dir_all(&target)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&target, [])?;
        }
        mounts.push(json!({"destination":host,"type":"bind","source":path.canonicalize()?,"options":["rbind","ro"]}));
    }
    std::fs::write(rootfs.join("runtime/opencoder-cli"), [])?;
    mounts.push(json!({"destination":"/runtime/opencoder-cli","type":"bind","source":cli,"options":["bind","ro"]}));
    let environment = process_environment(exclusions);
    let config = json!({"ociVersion":"1.0.0","hostname":"brain-activation","root":{"path":"rootfs","readonly":true},
        "process":{"terminal":false,"user":{"uid":0,"gid":0},"cwd":"/workspace","args":["/runtime/opencoder-cli","brain","activate-local","--context","/workspace/context.json","--config","/workspace/config.json","--output","/workspace/decision.json"],"env":environment},
        "mounts":mounts,"linux":{"namespaces":[{"type":"pid"},{"type":"ipc"},{"type":"uts"},{"type":"mount"}]}});
    opencoder_core::atomic_write_json(&bundle.join("config.json"), &config)?;
    Ok(())
}

fn process_environment(exclusions: &[(String, String)]) -> Vec<String> {
    let mut environment = vec!["PATH=/runtime:/usr/bin:/bin".into(), "HOME=/tmp".into()];
    environment.extend(
        exclusions
            .iter()
            .map(|(key, value)| format!("{key}={value}")),
    );
    environment
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_environment_carries_both_proxy_exclusions() {
        let exclusions = vec![
            ("NO_PROXY".into(), "model.internal".into()),
            ("no_proxy".into(), "other.internal".into()),
        ];
        assert_eq!(
            process_environment(&exclusions),
            [
                "PATH=/runtime:/usr/bin:/bin",
                "HOME=/tmp",
                "NO_PROXY=model.internal",
                "no_proxy=other.internal",
            ]
        );
    }
}

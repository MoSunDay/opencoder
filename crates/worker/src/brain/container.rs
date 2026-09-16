use crate::Worker;
use anyhow::{ensure, Context, Result};
use opencoder_core::{brain::*, Config};
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

pub async fn activate(
    worker: &Worker,
    config: &Config,
    context: &ActivationContext,
    cancel: CancellationToken,
) -> Result<ActivationDecision> {
    // Same external-provider injection seam as the native execution adapters.
    if let Some(client) = &worker.inner.client {
        return tokio::select! {
            _ = cancel.cancelled() => anyhow::bail!("brain activation cancelled"),
            result = opencoder_brain::activation::activate(context,client.as_ref(),config.model_id()) => result,
        };
    }
    ensure!(
        opencoder_dag_runtime::sandbox::runc::runc_available(),
        "Brain requires runc"
    );
    let cli = cli_path()?;
    let root = worker
        .inner
        .layout
        .execution_dir(opencoder_core::fleet::ExecutionKind::Brain, &context.run_id)?;
    let activation = root
        .join("activations")
        .join(context.activation.to_string());
    let bundle = worker
        .inner
        .layout
        .kind_root(opencoder_core::fleet::ExecutionKind::Brain)
        .join("bundles")
        .join(&context.run_id)
        .join(context.activation.to_string());
    std::fs::create_dir_all(&activation)?;
    private_json(&activation.join("context.json"), context)?;
    private_json(&activation.join("config.json"), config)?;
    write_bundle(&bundle, &activation, &cli)?;
    let id = format!("{}-a{}", context.run_id, context.activation);
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
    Ok(serde_json::from_slice(&bytes)?)
}

fn private_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    opencoder_core::atomic_write_json(path, &serde_json::to_value(value)?)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn write_bundle(bundle: &Path, activation: &Path, cli: &Path) -> Result<()> {
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
    let config = json!({"ociVersion":"1.0.0","hostname":"brain-activation","root":{"path":"rootfs","readonly":true},
        "process":{"terminal":false,"user":{"uid":0,"gid":0},"cwd":"/workspace","args":["/runtime/opencoder-cli","brain","activate-local","--context","/workspace/context.json","--config","/workspace/config.json","--output","/workspace/decision.json"],"env":["PATH=/runtime:/usr/bin:/bin","HOME=/tmp"]},
        "mounts":mounts,"linux":{"namespaces":[{"type":"pid"},{"type":"ipc"},{"type":"uts"},{"type":"mount"}]}});
    opencoder_core::atomic_write_json(&bundle.join("config.json"), &config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires workspace binaries and privileged runc; run explicitly on a node host"]
    async fn mounted_cli_finishes_fixed_activation_without_model_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("activation");
        std::fs::create_dir_all(&workspace).unwrap();
        let cli = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("opencoder-cli");
        assert!(cli.is_file(), "build workspace binaries first");
        let context = json!({"schema_version":2,"run_id":"brain-runc-smoke","activation":1,"control_epoch":1,"revision":1,"plan_ref":{"id":"plan-smoke","version":1},"plan":{"schema_version":2,"title":"Smoke","objective":"finite dispatch","instances":[],"inputs":{},"outputs":{},"routes":[],"entry":[]},"objective":"finite dispatch","phase":"running","inputs":{},"instances":[],"ready":["ready-a","ready-b"],"references":[],"capabilities":[],"routes":[]});
        private_json(&workspace.join("context.json"), &context).unwrap();
        private_json(&workspace.join("config.json"), &json!({})).unwrap();
        let bundle = dir.path().join("bundle");
        write_bundle(&bundle, &workspace, &cli).unwrap();
        let (code, output) = opencoder_dag_runtime::sandbox::runc::run_step_cancellable(
            &bundle,
            &format!("brain-smoke-{}", ulid::Ulid::new()),
            Some(20),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(code, 0, "{output}");
        let decision: ActivationDecision =
            serde_json::from_slice(&std::fs::read(workspace.join("decision.json")).unwrap())
                .unwrap();
        assert_eq!(decision.dispatch, vec!["ready-a", "ready-b"]);
        assert_eq!(decision.activation, 1);
        assert!(decision.plan.is_none());
    }
}

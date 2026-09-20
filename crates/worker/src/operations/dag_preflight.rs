use crate::Worker;
use anyhow::{bail, Result};
use opencoder_core::fleet::ExecutionKind;

pub(super) fn validate(
    worker: &Worker,
    config: &opencoder_core::Config,
    spec: &opencoder_dag::DagSpec,
    legacy: bool,
) -> Result<()> {
    if spec
        .steps
        .iter()
        .any(|step| crate::layout::reserved_execution_entry(&step.name))
    {
        bail!("DAG step name is reserved by the execution layout");
    }
    let workflow_root = if legacy {
        worker.inner.layout.checked_legacy_workflow_root()?
    } else {
        worker.inner.layout.checked_kind_root(ExecutionKind::Dag)?
    };
    // Wasm modules are pinned at accept so already-running runs keep
    // their frozen `_modules` copies through later pool publishes.
    crate::dag_wasm_pin::pin(config, spec, &workflow_root)?;
    if !spec.steps.iter().any(|step| {
        matches!(
            step.kind.executable(),
            opencoder_dag::StepKind::Wasm {
                sandbox: Some(opencoder_dag::SandboxMode::Runc),
                ..
            }
        ) || (config.dag.agent_sandbox == opencoder_core::config::AgentSandbox::Runc
            && matches!(
                step.kind.executable(),
                opencoder_dag::StepKind::Agent { .. }
            ))
    }) {
        return Ok(());
    }
    let rootfs = workflow_root.join("rootfs");
    if !std::fs::symlink_metadata(&rootfs).is_ok_and(|meta| meta.is_dir()) {
        bail!(
            "runc rootfs unavailable at {}; prepare a real directory before creating the execution",
            rootfs.display()
        );
    }
    if !opencoder_dag_runtime::sandbox::runc::runc_available() {
        bail!("runc executable unavailable for requested DAG sandbox");
    }
    if config.dag.agent_sandbox == opencoder_core::config::AgentSandbox::Runc {
        for step in &spec.steps {
            if let opencoder_dag::StepKind::Agent { agent, .. } = step.kind.executable() {
                opencoder_dag_runtime::sandbox::codex::resolve(
                    config,
                    agent.as_deref().unwrap_or("act"),
                    &rootfs,
                )?;
            }
        }
    }
    Ok(())
}

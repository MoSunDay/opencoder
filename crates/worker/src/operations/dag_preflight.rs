use crate::Worker;
use anyhow::{bail, Result};
use opencoder_core::fleet::ExecutionKind;

pub(super) fn validate(worker: &Worker, spec: &opencoder_dag::DagSpec, legacy: bool) -> Result<()> {
    if spec
        .steps
        .iter()
        .any(|step| crate::layout::reserved_execution_entry(&step.name))
    {
        bail!("DAG step name is reserved by the execution layout");
    }
    if !spec.steps.iter().any(|step| {
        matches!(
            &step.kind,
            opencoder_dag::StepKind::Wasm {
                sandbox: Some(opencoder_dag::SandboxMode::Runc),
                ..
            }
        )
    }) {
        return Ok(());
    }
    let workflow_root = if legacy {
        worker.inner.layout.checked_legacy_workflow_root()?
    } else {
        worker.inner.layout.checked_kind_root(ExecutionKind::Dag)?
    };
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
    Ok(())
}

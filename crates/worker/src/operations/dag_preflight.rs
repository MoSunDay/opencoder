use crate::Worker;
#[cfg(not(windows))]
use anyhow::ensure;
use anyhow::Result;
use opencoder_core::fleet::Assignment;

pub(super) fn validate(
    worker: &Worker,
    config: &opencoder_core::Config,
    spec: &opencoder_dag::DagSpec,
    assignment: &Assignment,
    legacy: bool,
) -> Result<()> {
    #[cfg(windows)]
    {
        let _ = (worker, config, spec, assignment, legacy);
        anyhow::bail!("DAG execution requires Linux");
    }
    #[cfg(not(windows))]
    {
        ensure!(
            !legacy,
            "old DAG checkpoints cannot resume; start a new container run"
        );
        if let Some(arguments) = assignment.request.input.get("args") {
            let arguments: Vec<String> = serde_json::from_value(arguments.clone())?;
            ensure!(
                arguments.iter().all(|argument| !argument.contains('\0')),
                "DAG arguments cannot contain NUL"
            );
        }
        opencoder_dag_runtime::sandbox::run::preflight(config)?;
        let parent = crate::layout::dag::parent(worker, config, assignment)?;
        let root = parent.join(&assignment.index.id);
        if !root.join("resources.json").exists()
            && spec.steps.iter().any(|step| {
                matches!(
                    step.kind.executable(),
                    opencoder_dag::StepKind::Binary { .. }
                )
            })
        {
            let source = config
                .dag
                .binary_dir
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("DAG binary pool is required"))?;
            opencoder_dag_runtime::nfs::read_only_mount(source)?;
        }
        for step in &spec.steps {
            if let opencoder_dag::StepKind::Agent { agent, .. } = step.kind.executable() {
                opencoder_dag_runtime::sandbox::codex::resolve(
                    config,
                    agent.as_deref().unwrap_or("act"),
                    config.dag.rootfs_dir.as_ref().unwrap(),
                )?;
            }
        }
        opencoder_dag_runtime::resources::freeze(&root, config, spec)
    }
}

use crate::service::Deps;
use anyhow::{ensure, Context, Result};
use opencoder_core::Config;
use opencoder_dag::{DagClaimedRun, DagSpec};
use opencoder_store::{
    ProjectExecutorKind, ProjectTodoRunPatch, ProjectTodoRunRecord, ProjectTodoRunStatus,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
struct Accepted {
    workdir: PathBuf,
    config: Config,
    run: DagClaimedRun,
}

fn read(root: &Path, deps: &Deps, run_id: &str) -> Result<Accepted> {
    use std::{io::Read, os::unix::fs::OpenOptionsExt};
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(root.join("project-dag.json"))?;
    ensure!(
        file.metadata()?.is_file(),
        "project DAG receipt must be a regular file"
    );
    let mut bytes = Vec::new();
    (&mut file)
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 4 * 1024 * 1024,
        "project DAG receipt exceeds limit"
    );
    let accepted: Accepted = serde_json::from_slice(&bytes)?;
    ensure!(
        accepted.workdir == deps.workdir && accepted.run.run_id == run_id,
        "project DAG recovery ownership mismatch"
    );
    Ok(accepted)
}

fn validate_root(root: &Path, run_id: &str) -> Result<()> {
    ensure!(
        root.is_absolute() && root.file_name().and_then(|name| name.to_str()) == Some(run_id),
        "invalid project DAG recovery path"
    );
    let mut current = Some(root);
    while let Some(path) = current {
        ensure!(
            std::fs::symlink_metadata(path)?.is_dir(),
            "project DAG path must be a real directory"
        );
        current = path.parent();
    }
    Ok(())
}

pub(crate) fn restored(
    deps: &Deps,
    record: &ProjectTodoRunRecord,
) -> Result<Option<(Config, DagClaimedRun, PathBuf, bool)>> {
    let Some(path) = record.output_ref.as_deref() else {
        return Ok(None);
    };
    let root = PathBuf::from(path);
    validate_root(&root, &record.id)?;
    let accepted = read(&root, deps, &record.id)?;
    Ok(Some((accepted.config, accepted.run, root, true)))
}

pub(crate) async fn prepare(
    deps: &Deps,
    record: &ProjectTodoRunRecord,
    config: Config,
    spec: DagSpec,
    dag_id: String,
) -> Result<(Config, DagClaimedRun, PathBuf, bool)> {
    let data = config
        .dag
        .data_dir
        .as_ref()
        .cloned()
        .unwrap_or_else(|| opencoder_core::data_dir_for(&deps.workdir).join("dag/runs"));
    let parent = opencoder_dag_runtime::layout::run_parent(&data, &dag_id, record.created_at)?;
    let root = record
        .output_ref
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or(parent.join(&record.id));
    std::fs::create_dir_all(&root)?;
    validate_root(&root, &record.id)?;
    let receipt = root.join("project-dag.json");
    let resume = receipt.exists();
    let accepted = if resume {
        read(&root, deps, &record.id)?
    } else {
        ensure!(record.output_ref.is_none(), "project DAG receipt missing");
        opencoder_dag_runtime::sandbox::run::preflight(&config)?;
        for step in &spec.steps {
            match step.kind.executable() {
                opencoder_dag::StepKind::Binary { .. } => {
                    opencoder_dag_runtime::nfs::read_only_mount(
                        config
                            .dag
                            .binary_dir
                            .as_deref()
                            .context("DAG binary pool required")?,
                    )?;
                }
                opencoder_dag::StepKind::Agent { agent, .. } => {
                    if let Some(source) = config.agent.agents_dir.as_deref() {
                        opencoder_dag_runtime::nfs::read_only_mount(source)?;
                    }
                    opencoder_dag_runtime::sandbox::codex::resolve(
                        &config,
                        agent.as_deref().unwrap_or("act"),
                        config.dag.rootfs_dir.as_ref().unwrap(),
                    )?;
                }
                _ => unreachable!(),
            }
        }
        opencoder_dag_runtime::resources::freeze(&root, &config, &spec)?;
        let accepted = Accepted {
            workdir: deps.workdir.clone(),
            config,
            run: DagClaimedRun {
                run_id: record.id.clone(),
                dag_id,
                spec,
                created_at: record.created_at,
            },
        };
        opencoder_core::atomic_write(&receipt, &serde_json::to_vec(&accepted)?)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&receipt, std::fs::Permissions::from_mode(0o600))?;
        accepted
    };
    ensure!(
        deps.projects
            .patch_todo_run_when(
                &record.id,
                ProjectTodoRunStatus::Running,
                &ProjectTodoRunPatch {
                    output_ref: Some(root.display().to_string()),
                    ..Default::default()
                },
                opencoder_core::message::now_ms()
            )
            .await?,
        "project DAG run is no longer running"
    );
    Ok((accepted.config, accepted.run, root, resume))
}

pub(crate) async fn cleanup(deps: &Deps, run: &ProjectTodoRunRecord) -> Result<()> {
    if run.executor_kind != ProjectExecutorKind::Dag || run.status != ProjectTodoRunStatus::Running
    {
        return Ok(());
    }
    let Some(path) = run.output_ref.as_deref() else {
        return Ok(());
    };
    let root = Path::new(path);
    validate_root(root, &run.id)?;
    read(root, deps, &run.id)?;
    opencoder_dag_runtime::sandbox::run::cleanup_run(root, &run.id).await
}

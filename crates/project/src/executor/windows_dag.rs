//! Windows operator links project types without a container executor.
use crate::service::Deps;
use opencoder_store::{ProjectExecutorKind, ProjectTodoRunRecord, ProjectTodoRunStatus};
pub(crate) async fn cleanup(_deps: &Deps, run: &ProjectTodoRunRecord) -> anyhow::Result<()> {
    anyhow::ensure!(
        run.executor_kind != ProjectExecutorKind::Dag
            || run.status != ProjectTodoRunStatus::Running,
        "DAG recovery requires Linux"
    );
    Ok(())
}

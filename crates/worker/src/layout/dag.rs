use crate::{journal::Record, Worker};
use anyhow::{ensure, Context, Result};
use opencoder_core::{fleet::Assignment, Config};
use std::path::PathBuf;

pub(crate) fn parent(worker: &Worker, config: &Config, assignment: &Assignment) -> Result<PathBuf> {
    let dag = assignment
        .request
        .target
        .as_deref()
        .unwrap_or(&assignment.index.id);
    let data = config.dag.data_dir.clone().unwrap_or_else(|| {
        worker
            .inner
            .layout
            .kind_root(opencoder_core::fleet::ExecutionKind::Dag)
            .join("runs")
    });
    opencoder_dag_runtime::layout::run_parent(&data, dag, assignment.index.created_at)
}

pub(crate) fn accepted_parent(record: &Record) -> Result<PathBuf> {
    let parent = record.annotations["dag_parent"]
        .as_str()
        .context("DAG has no accepted container workspace; start a new run")?;
    let parent = PathBuf::from(parent);
    ensure!(parent.is_absolute(), "accepted DAG parent must be absolute");
    Ok(parent)
}

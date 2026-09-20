use crate::{journal::Record, Worker};
use anyhow::Result;
use opencoder_core::fleet::ExecutionKind;
use std::{path::PathBuf, sync::Arc};

/// Effective opencoder workspace for sessions this node runs: the
/// scheduling-configured workdir when set, else the node's startup workdir.
pub(crate) fn node_workdir(worker: &Worker) -> PathBuf {
    match worker.inner.scheduling.get().workdir {
        Some(dir) => PathBuf::from(dir),
        None => worker.inner.state.workdir.clone(),
    }
}

fn with_workdir(
    state: &opencoder_web::AppState,
    workdir: PathBuf,
    config_home: Option<PathBuf>,
) -> Arc<opencoder_web::AppState> {
    Arc::new(opencoder_web::AppState {
        store: state.store.clone(),
        workdir,
        config_home,
        handles: state.handles.clone(),
        nodes: state.nodes.clone(),
        controls: state.controls.clone(),
        project: state.project.clone(),
        team: state.team.clone(),
        brain: state.brain.clone(),
        client_override: state.client_override.clone(),
    })
}

/// Resolve the operator execution's materialized (home, workspace), if any.
fn operator_env(
    worker: &Worker,
    kind: ExecutionKind,
    id: &str,
) -> Result<Option<(PathBuf, PathBuf)>> {
    crate::operations::operator_env::resolve(&worker.inner.layout, kind, id)
}

pub fn for_record(worker: &Worker, record: &Record) -> Result<PathBuf> {
    // Operator executions own a per-execution workspace (isolation): use it
    // whenever the materialized pair exists; legacy or crash-fallback
    // records keep the node workdir so resumed behavior is unchanged.
    if record.assignment.request.kind == ExecutionKind::Operator {
        if let Some((_home, workspace)) =
            operator_env(worker, ExecutionKind::Operator, &record.assignment.index.id)?
        {
            return Ok(workspace);
        }
        return Ok(node_workdir(worker));
    }
    if record.assignment.request.input.get("_brain").is_none()
        && record
            .assignment
            .request
            .input
            .get("brain_scheduler")
            .is_none()
    {
        return Ok(node_workdir(worker));
    }
    let path = worker
        .inner
        .layout
        .execution_dir(record.assignment.index.kind, &record.assignment.index.id)?
        .join("workspace");
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

/// Session workdir + config-home redirect for an agent-workload record:
/// operator executions get their frozen pair; every other kind keeps the
/// existing `for_record` resolution with live config discovery.
pub(crate) fn session_dirs(
    worker: &Worker,
    record: &Record,
) -> Result<(PathBuf, Option<PathBuf>)> {
    if record.assignment.request.kind == ExecutionKind::Operator {
        if let Some((home, workspace)) =
            operator_env(worker, ExecutionKind::Operator, &record.assignment.index.id)?
        {
            return Ok((workspace, Some(home)));
        }
    }
    Ok((for_record(worker, record)?, None))
}

pub async fn native_state(worker: &Worker, path: &str) -> Result<Arc<opencoder_web::AppState>> {
    let state = &worker.inner.state;
    if let Some(id) = path
        .strip_prefix("/api/sessions/")
        .and_then(|tail| tail.split('/').next())
    {
        let journal = worker.inner.journal.lock().await;
        if let Some(record) = journal.records.get(id) {
            // Operator sessions reload config from the execution's frozen
            // home snapshot and run tools inside its workspace.
            if record.assignment.request.kind == ExecutionKind::Operator {
                if let Some((home, workspace)) =
                    operator_env(worker, ExecutionKind::Operator, &record.assignment.index.id)?
                {
                    return Ok(with_workdir(state, workspace, Some(home)));
                }
                // Unmaterialized (legacy/fallback) operator record: the
                // node-level state below keeps prior behavior.
            } else if record.assignment.request.input.get("_brain").is_some()
                || record
                    .assignment
                    .request
                    .input
                    .get("brain_scheduler")
                    .is_some()
            {
                return Ok(with_workdir(state, for_record(worker, record)?, None));
            }
        }
    }
    let workdir = node_workdir(worker);
    if workdir == state.workdir {
        return Ok(state.clone());
    }
    Ok(with_workdir(state, workdir, None))
}

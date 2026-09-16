use crate::{journal::Record, Worker};
use anyhow::Result;
use std::{path::PathBuf, sync::Arc};

/// Effective opencoder workspace for sessions this node runs: the
/// scheduling-configured workdir when set, else the node's startup workdir.
pub(crate) fn node_workdir(worker: &Worker) -> PathBuf {
    match worker.inner.scheduling.get().workdir {
        Some(dir) => PathBuf::from(dir),
        None => worker.inner.state.workdir.clone(),
    }
}

fn with_workdir(state: &opencoder_web::AppState, workdir: PathBuf) -> Arc<opencoder_web::AppState> {
    Arc::new(opencoder_web::AppState {
        store: state.store.clone(),
        workdir,
        handles: state.handles.clone(),
        nodes: state.nodes.clone(),
        controls: state.controls.clone(),
        project: state.project.clone(),
        team: state.team.clone(),
        brain: state.brain.clone(),
        client_override: state.client_override.clone(),
    })
}

pub fn for_record(worker: &Worker, record: &Record) -> Result<PathBuf> {
    if record.assignment.request.input.get("_brain").is_none() {
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

pub async fn native_state(worker: &Worker, path: &str) -> Result<Arc<opencoder_web::AppState>> {
    let state = &worker.inner.state;
    if let Some(id) = path
        .strip_prefix("/api/sessions/")
        .and_then(|tail| tail.split('/').next())
    {
        let journal = worker.inner.journal.lock().await;
        if let Some(record) = journal
            .records
            .get(id)
            .filter(|r| r.assignment.request.input.get("_brain").is_some())
        {
            return Ok(with_workdir(state, for_record(worker, record)?));
        }
    }
    let workdir = node_workdir(worker);
    if workdir == state.workdir {
        return Ok(state.clone());
    }
    Ok(with_workdir(state, workdir))
}

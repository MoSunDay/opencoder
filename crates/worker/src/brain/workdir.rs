use crate::{journal::Record, Worker};
use anyhow::Result;
use std::{path::PathBuf, sync::Arc};

pub fn for_record(worker: &Worker, record: &Record) -> Result<PathBuf> {
    if record.assignment.request.input.get("_brain").is_none() {
        return Ok(worker.inner.state.workdir.clone());
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
    let Some(id) = path
        .strip_prefix("/api/sessions/")
        .and_then(|tail| tail.split('/').next())
    else {
        return Ok(worker.inner.state.clone());
    };
    let journal = worker.inner.journal.lock().await;
    let Some(record) = journal
        .records
        .get(id)
        .filter(|r| r.assignment.request.input.get("_brain").is_some())
    else {
        return Ok(worker.inner.state.clone());
    };
    let state = &worker.inner.state;
    Ok(Arc::new(opencoder_web::AppState {
        store: state.store.clone(),
        workdir: for_record(worker, record)?,
        handles: state.handles.clone(),
        nodes: state.nodes.clone(),
        controls: state.controls.clone(),
        project: state.project.clone(),
        team: state.team.clone(),
        brain: state.brain.clone(),
        client_override: state.client_override.clone(),
    }))
}

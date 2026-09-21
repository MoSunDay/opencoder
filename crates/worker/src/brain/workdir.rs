use crate::{journal::Record, Worker};
use anyhow::Result;
use opencoder_core::Config;
use std::{path::PathBuf, sync::Arc};

#[cfg(test)]
mod tests;

pub(crate) fn node_workdir(worker: &Worker) -> PathBuf {
    worker
        .inner
        .scheduling
        .get()
        .workdir
        .map(PathBuf::from)
        .unwrap_or_else(|| worker.inner.state.workdir.clone())
}

fn managed(record: &Record) -> bool {
    let input = &record.assignment.request.input;
    input.get("_brain").is_some() || input.get("brain_scheduler").is_some()
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

fn operator_env(worker: &Worker, record: &Record) -> Result<Option<(PathBuf, PathBuf)>> {
    crate::operations::operator_env::resolve(
        &worker.inner.layout,
        record.assignment.request.kind,
        &record.assignment.index.id,
        record.annotations.get("operator_environment_version"),
    )
}

pub fn for_record(worker: &Worker, record: &Record) -> Result<PathBuf> {
    if let Some((_, workspace)) = operator_env(worker, record)? {
        return Ok(workspace);
    }
    // Historical Brain Operators keep their existing isolated workspace.
    if !managed(record) {
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

/// Skill-discovery root for an execution: the frozen operator pool under
/// the execution home, or `None` (node-level discovery) for every other
/// kind. Scoped into the workload via `skill::with_execution`.
pub(crate) fn execution_skill_root(worker: &Worker, record: &Record) -> Option<PathBuf> {
    operator_env(worker, record)
        .ok()
        .flatten()
        .map(|(home, _)| home.join(".opencoder").join("skills"))
}

pub(crate) fn session_dirs(worker: &Worker, record: &Record) -> Result<(PathBuf, Option<PathBuf>)> {
    if let Some((home, workspace)) = operator_env(worker, record)? {
        return Ok((workspace, Some(home)));
    }
    Ok((for_record(worker, record)?, None))
}

/// Reuse frozen admission settings for every follow-up and explicit resume.
pub(crate) fn execution_config(worker: &Worker, record: &Record) -> Result<Option<Config>> {
    if let Some((home, workspace)) = operator_env(worker, record)? {
        // Versioned operator executions are "snapshot is final": env
        // overlays never re-enter after creation (see
        // `Config::load_with_home_frozen`).
        return Ok(Some(Config::load_with_home_frozen(
            &workspace,
            Some(&home),
        )?));
    }
    if managed(record) {
        return record
            .queue
            .as_ref()
            .map(|queued| queued.config.clone())
            .map(Ok)
            .unwrap_or_else(|| worker.configuration())
            .map(Some);
    }
    Ok(None)
}

pub async fn native_state(
    worker: &Worker,
    path: &str,
) -> Result<(Arc<opencoder_web::AppState>, Option<Config>)> {
    let state = &worker.inner.state;
    if let Some(id) = path
        .strip_prefix("/api/sessions/")
        .and_then(|tail| tail.split('/').next())
    {
        let journal = worker.inner.journal.lock().await;
        if let Some(record) = journal.records.get(id) {
            if let Some((home, workspace)) = operator_env(worker, record)? {
                return Ok((with_workdir(state, workspace, Some(home)), None));
            }
            if managed(record) {
                // Keep admitted model/provider/AP settings in process. They do
                // not belong in the managed child workspace.
                let config = record
                    .queue
                    .as_ref()
                    .map(|queued| queued.config.clone())
                    .map(Ok)
                    .unwrap_or_else(|| worker.configuration())?;
                return Ok((
                    with_workdir(state, for_record(worker, record)?, None),
                    Some(config),
                ));
            }
        }
    }
    let workdir = node_workdir(worker);
    if workdir == state.workdir {
        return Ok((state.clone(), None));
    }
    Ok((with_workdir(state, workdir, None), None))
}

//! Seal the observable boundary of a lost attempt before a session can resume.
use crate::service::Deps;
use anyhow::{Context, Result};
use opencoder_store::ProjectTodoRunRecord;
use serde_json::{json, Value};

pub async fn partial_manifest(deps: &Deps, run: &ProjectTodoRunRecord) -> Result<Option<String>> {
    let Some(raw) = &run.trace_manifest else {
        return Ok(None);
    };
    let mut trace: Value = serde_json::from_str(raw)?;
    if trace["complete"] == true || trace.get("messages_through").is_some() {
        return Ok(None);
    }
    let sid = run
        .session_id
        .as_deref()
        .context("partial trace session missing")?;
    trace["messages_through"] = json!(deps.store.last_message_seq(sid).await?);
    trace["events_through"] = json!(deps.store.last_event_seq(sid).await?);
    trace["interrupted"] = json!(true);
    let root = super::archive::run_root(&super::root(deps), &run.id)?;
    let mut files = Vec::new();
    let mut artifacts = Vec::new();
    let mut events = 0;
    let mut calls = 0;
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("event-") && name.ends_with(".meta.json") {
            events += 1;
        }
        if name.starts_with("request-") {
            calls += 1;
        }
        if name.starts_with("request-") || name.starts_with("response-") {
            files.push(name.clone());
        }
        if name.starts_with("artifact-") && name.ends_with(".json") {
            artifacts.push(serde_json::from_slice::<Value>(&std::fs::read(
                entry.path(),
            )?)?);
        }
    }
    trace["files"] = json!(files);
    trace["artifacts"] = json!(artifacts);
    trace["event_count"] = json!(events);
    trace["model_calls"] = json!(calls);
    let prior = trace["children_before"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    trace["children"] = json!(deps.store.list_subagent_tasks(sid).await?.into_iter()
        .filter(|task| !prior.contains(&json!(task.task_id)))
        .map(|task| json!({"task_id":task.task_id,"child_session_id":task.child_session_id,"agent":task.agent,"status":task.status})).collect::<Vec<_>>());
    Ok(Some(trace.to_string()))
}

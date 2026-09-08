//! A child link is complete only when its session and task record are durable.
use crate::service::Deps;
use anyhow::{Context, Result};
use opencoder_session::SessionEvent;
use serde_json::{json, Value};

pub fn started(event: &SessionEvent, output: &mut Vec<(String, String)>) {
    match event {
        SessionEvent::SubagentStart {
            id,
            child_session_id,
            ..
        } => output.push((id.clone(), child_session_id.clone())),
        SessionEvent::SubagentChild { ev, .. } => started(ev, output),
        _ => {}
    }
}
pub async fn links(
    deps: &Deps,
    parent: &str,
    before: &[String],
    expected: &[(String, String)],
) -> Result<Vec<Value>> {
    let mut parents = vec![parent.to_string()];
    let mut links = Vec::new();
    while let Some(id) = parents.pop() {
        for task in deps.store.list_subagent_tasks(&id).await? {
            if id == parent && before.contains(&task.task_id) {
                continue;
            }
            anyhow::ensure!(
                !parents.contains(&task.child_session_id)
                    && !links
                        .iter()
                        .any(|link: &Value| link["child_session_id"] == task.child_session_id),
                "cyclic child session references"
            );
            deps.store
                .get_session(&task.child_session_id)
                .await?
                .context("project child session missing")?;
            parents.push(task.child_session_id.clone());
            links.push(json!({"task_id":task.task_id,"parent_session_id":id,"child_session_id":task.child_session_id,"agent":task.agent,"status":task.status}));
        }
    }
    for (task, session) in expected {
        anyhow::ensure!(
            links
                .iter()
                .any(|link| link["task_id"] == *task && link["child_session_id"] == *session),
            "project child task record missing: {task}"
        );
    }
    Ok(links)
}

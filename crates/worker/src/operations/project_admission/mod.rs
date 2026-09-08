//! Project snapshots and durable run admission, shared by create and commands.
use crate::Worker;
use anyhow::{Context, Result};
use opencoder_core::fleet::*;
use opencoder_store::*;
use serde_json::{json, Value};

pub fn kind(action: &str) -> ProjectTodoRunKind {
    if action == "execute" {
        ProjectTodoRunKind::Execute
    } else {
        ProjectTodoRunKind::Plan
    }
}
pub fn request(action: &str, input: &Value) -> Value {
    let mut input = input.as_object().cloned().unwrap_or_default();
    for key in ["snapshot", "brain", "run_id", "action", "node_id"] {
        input.remove(key);
    }
    json!({"action":action,"input":input})
}
pub fn ensure_id(input: &mut Value) -> Result<String> {
    if input.is_null() {
        *input = json!({});
    }
    let id = match input.get("run_id") {
        Some(Value::String(id)) => id.clone(),
        None => format!("prun-{}", ulid::Ulid::new()),
        _ => anyhow::bail!("invalid project run id"),
    };
    anyhow::ensure!(
        id.starts_with("prun-") && valid_id(&id),
        "invalid project run id"
    );
    input["run_id"] = json!(id);
    Ok(id)
}
pub async fn existing(
    worker: &Worker,
    todo: &str,
    action: &str,
    input: &Value,
) -> Result<Option<ProjectTodoRunRecord>> {
    let Some(id) = input["run_id"].as_str() else {
        return Ok(None);
    };
    worker
        .inner
        .state
        .project
        .accepted_attempt(todo, kind(action), id, &request(action, input))
        .await
}
pub fn receipt(worker: &Worker, owner: &str, run: &ProjectTodoRunRecord) -> RpcReply {
    RpcReply::ok(
        json!({"id":owner,"kind":"project","run_id":run.id,"node_id":worker.inner.registration.id,"status":match run.status {ProjectTodoRunStatus::Running=>"running",ProjectTodoRunStatus::Done=>"idle",ProjectTodoRunStatus::Failed=>"error",ProjectTodoRunStatus::Cancelled=>"cancelled"},"run_status":run.status}),
    )
}
pub async fn reserve(
    worker: &Worker,
    assignment: &Assignment,
    action: &str,
    input: &Value,
    config: &opencoder_core::Config,
) -> Result<ProjectTodoRunRecord> {
    let id = input["run_id"].as_str().context("project run id missing")?;
    let snapshot = assignment
        .definition
        .as_ref()
        .context("project snapshot missing")?;
    let todo: ProjectTodoRecord = serde_json::from_value(snapshot["todo"].clone())?;
    if let Some(run) = existing(worker, &todo.id, action, input).await? {
        return Ok(run);
    }
    // Only a new attempt reaches this path. An interrupted old driver is closed
    // explicitly before a new attempt; its input and partial trace remain intact.
    for run in worker
        .inner
        .state
        .project
        .require()?
        .projects
        .list_todo_runs(&todo.id)
        .await?
    {
        if run.status == ProjectTodoRunStatus::Running {
            worker.inner.state.project.cancel(&run.id).await?;
        }
    }
    sync_snapshot(worker, snapshot, action).await?;
    let override_ = input
        .get("brain")
        .filter(|v| !v.is_null())
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()?;
    opencoder_core::agent::scope::with_root(
        config.agent.agents_dir.clone(),
        worker.inner.state.project.reserve_attempt(
            &todo.id,
            kind(action),
            id,
            override_,
            request(action, input),
        ),
    )
    .await
}
async fn sync_snapshot(worker: &Worker, snapshot: &Value, action: &str) -> Result<()> {
    let projects = worker.inner.state.project.require()?.projects.clone();
    let todo: ProjectTodoRecord = serde_json::from_value(snapshot["todo"].clone())?;
    if action == "plan" || projects.get_todo(&todo.id).await?.is_none() {
        for goal in serde_json::from_value::<Vec<ProjectGoalRecord>>(snapshot["goals"].clone())? {
            if projects.list_goals().await?.iter().any(|g| g.id == goal.id) {
                projects
                    .patch_goal(
                        &goal.id,
                        &ProjectGoalPatch {
                            title: Some(goal.title),
                            detail_md: goal.detail_md,
                            ..Default::default()
                        },
                        opencoder_core::message::now_ms(),
                    )
                    .await?;
            } else {
                projects.create_goal(&goal).await?;
            }
        }
        for m in
            serde_json::from_value::<Vec<ProjectMilestoneRecord>>(snapshot["milestones"].clone())?
        {
            if projects
                .list_milestones(None)
                .await?
                .iter()
                .any(|old| old.id == m.id)
            {
                projects
                    .patch_milestone(
                        &m.id,
                        &ProjectMilestonePatch {
                            title: Some(m.title),
                            goal_id: Some(m.goal_id),
                            detail_md: m.detail_md,
                            ..Default::default()
                        },
                        opencoder_core::message::now_ms(),
                    )
                    .await?;
            } else {
                projects.create_milestone(&m).await?;
            }
        }
    }
    if projects.get_todo(&todo.id).await?.is_none() {
        projects.create_todo(&todo).await?;
    } else {
        let mut patch = ProjectTodoPatch {
            agent: Some(todo.agent),
            executor_kind: Some(todo.executor_kind),
            executor_ref: Some(todo.executor_ref),
            executor_spec: Some(todo.executor_spec),
            ..Default::default()
        };
        if action == "plan" {
            patch.title = Some(todo.title);
            patch.draft = Some(todo.draft);
            patch.milestone_id = Some(todo.milestone_id);
        }
        projects
            .patch_todo(&todo.id, &patch, opencoder_core::message::now_ms())
            .await?;
    }
    Ok(())
}

pub fn error_reply(error: anyhow::Error) -> RpcReply {
    let message = format!("{error:#}");
    let status = if message.contains("no plan")
        || message.contains("is running")
        || message.contains("already accepted")
    {
        409
    } else {
        500
    };
    RpcReply::error(status, message)
}

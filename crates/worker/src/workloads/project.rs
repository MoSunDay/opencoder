use crate::{journal::Record, Worker};
use anyhow::Result;
use opencoder_core::fleet::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

/// The control-plane brain pre-resolution riding the execution input: fresh
/// submits carry it in `request.input.brain`, re-execute commands in
/// `record.result.brain`. Non-brain todos (and brain todos the control
/// plane left unresolved) yield None — the service falls back to the todo's
/// own executor fields.
fn brain_override(record: &Record) -> Option<opencoder_project::service::ExecutorOverride> {
    record
        .result
        .get("brain")
        .filter(|v| !v.is_null())
        .or_else(|| {
            record
                .assignment
                .request
                .input
                .get("brain")
                .filter(|v| !v.is_null())
        })
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

pub(super) async fn run(
    worker: &Worker,
    record: &Record,
    cancel: CancellationToken,
) -> Result<(ExecutionStatus, Value)> {
    let snapshot = record
        .result
        .get("next_snapshot")
        .or(record.assignment.definition.as_ref())
        .ok_or_else(|| anyhow::anyhow!("project snapshot missing"))?;
    let deps = worker.inner.state.project.require()?;
    let todo: opencoder_store::ProjectTodoRecord =
        serde_json::from_value(snapshot["todo"].clone())?;
    for goal in serde_json::from_value::<Vec<opencoder_store::ProjectGoalRecord>>(
        snapshot["goals"].clone(),
    )? {
        if !deps
            .projects
            .list_goals()
            .await?
            .iter()
            .any(|g| g.id == goal.id)
        {
            deps.projects.create_goal(&goal).await?;
        }
    }
    for milestone in serde_json::from_value::<Vec<opencoder_store::ProjectMilestoneRecord>>(
        snapshot["milestones"].clone(),
    )? {
        if !deps
            .projects
            .list_milestones(None)
            .await?
            .iter()
            .any(|m| m.id == milestone.id)
        {
            deps.projects.create_milestone(&milestone).await?;
        }
    }
    if deps.projects.get_todo(&todo.id).await?.is_none() {
        deps.projects.create_todo(&todo).await?;
    } else if record.result.get("next_snapshot").is_some() {
        deps.projects
            .patch_todo(
                &todo.id,
                &opencoder_store::ProjectTodoPatch {
                    draft: Some(todo.draft.clone()),
                    title: Some(todo.title.clone()),
                    agent: Some(todo.agent.clone()),
                    // The snapshot todo is authoritative for the executor
                    // triple too (control-plane writes may have changed it
                    // since this node last saw the todo).
                    executor_kind: Some(todo.executor_kind),
                    executor_ref: Some(todo.executor_ref.clone()),
                    executor_spec: Some(todo.executor_spec.clone()),
                    ..Default::default()
                },
                opencoder_core::message::now_ms(),
            )
            .await?;
    }
    let action = record.result["next_action"]
        .as_str()
        .or(record.assignment.request.input["action"].as_str())
        .unwrap_or("plan");
    for run in deps.projects.list_todo_runs(&todo.id).await? {
        if run.status == opencoder_store::ProjectTodoRunStatus::Running {
            worker.inner.state.project.cancel(&run.id).await?;
        }
    }
    let run_id = if action == "execute" {
        // Brain todos arrive pre-resolved by the control plane: fresh
        // submits carry the override in request.input, re-execute commands
        // in record.result (stored beside next_action/next_snapshot).
        let override_ = brain_override(record);
        worker
            .inner
            .state
            .project
            .start_execute_with(&todo.id, override_)
            .await?
    } else {
        worker.inner.state.project.start_plan(&todo.id).await?
    };
    worker
        .inner
        .journal
        .lock()
        .await
        .set_active_project_run(&record.assignment.index.id, &run_id)?;
    opencoder_session::loop_registry::notify_change();
    loop {
        if cancel.is_cancelled() {
            worker.inner.state.project.cancel(&run_id).await?;
        }
        let run = deps
            .projects
            .get_todo_run(&run_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("project run disappeared"))?;
        match run.status {
            opencoder_store::ProjectTodoRunStatus::Running => {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await
            }
            opencoder_store::ProjectTodoRunStatus::Done => {
                return Ok((ExecutionStatus::Idle, json!({"run_id":run_id,"run":run})))
            }
            _ => {
                return Ok((
                    if cancel.is_cancelled() {
                        ExecutionStatus::Cancelled
                    } else {
                        ExecutionStatus::Error
                    },
                    json!({"run_id":run_id,"run":run}),
                ))
            }
        }
    }
}

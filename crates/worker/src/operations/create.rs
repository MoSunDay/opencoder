use crate::{journal::Record, lifecycle::Lifecycle, Worker};
use anyhow::{bail, Result};
use opencoder_core::{fleet::*, Config};
use serde_json::{json, Value};

pub(super) use super::launch::{launch, launch_locked, LaunchOutcome};

pub(super) async fn create(worker: &Worker, assignment: Assignment) -> Result<RpcReply> {
    let _gate = worker.inner.admission.lock().await;
    if let Err(error) = assignment.request.validate() {
        return Ok(RpcReply::error(400, error));
    }
    if assignment.request.kind == ExecutionKind::System {
        return Ok(RpcReply::error(
            400,
            "system team execution is retired; use explicit node maintenance",
        ));
    }
    if assignment.index.node_id != worker.inner.registration.id
        || assignment.index.id != assignment.request.id
        || assignment.index.kind != assignment.request.kind
    {
        return Ok(RpcReply::error(
            409,
            "assignment ownership or kind mismatch",
        ));
    }
    if let Some(existing) = worker
        .inner
        .journal
        .lock()
        .await
        .records
        .get(&assignment.index.id)
        .cloned()
    {
        if existing.assignment.request != assignment.request {
            return Ok(RpcReply::error(
                409,
                "execution id already accepted with different input",
            ));
        }
        return Ok(RpcReply::ok(json!(existing.assignment.index)));
    }
    if let Some(error) = worker.admission_error() {
        return Ok(RpcReply::error(503, error));
    }
    if worker
        .inner
        .layout
        .execution_dir(assignment.index.kind, &assignment.index.id)?
        .exists()
    {
        return Ok(RpcReply::error(
            409,
            "execution directory exists without a durable journal record",
        ));
    }
    if assignment.definition.is_none()
        && !matches!(
            assignment.request.kind,
            ExecutionKind::Agent | ExecutionKind::Maintenance
        )
    {
        return Ok(RpcReply::error(
            428,
            "execution not yet accepted; definition snapshot required",
        ));
    }
    if !worker
        .inner
        .registration
        .kinds
        .contains(&assignment.request.kind)
    {
        return Ok(RpcReply::error(400, "unsupported execution kind"));
    }
    let permit = match worker.inner.slots.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return Ok(RpcReply::error(429, "node execution capacity exhausted")),
    };
    let config = match prepare(worker, &assignment, false) {
        Ok(config) => config,
        Err(error) => {
            return Ok(RpcReply::error(
                400,
                format!("execution preflight: {error:#}"),
            ))
        }
    };
    let mut assignment = assignment;
    assignment.index.status = ExecutionStatus::Pending;
    let lifecycle = if assignment.index.kind == ExecutionKind::Todos {
        Lifecycle::accepted_todo()
    } else {
        Lifecycle::default()
    };
    let record = Record {
        assignment,
        result: Value::Null,
        error: None,
        events: vec![],
        lifecycle,
    };
    worker.inner.journal.lock().await.save(record.clone())?;
    let _ = launch(worker.clone(), record.clone(), config, permit, false).await?;
    Ok(RpcReply::ok(json!(
        worker.inner.journal.lock().await.records[&record.assignment.index.id]
            .assignment
            .index
    )))
}

pub(super) fn prepare(worker: &Worker, assignment: &Assignment, legacy: bool) -> Result<Config> {
    if let Some(error) = worker.inner.persistence_error.lock().unwrap().as_ref() {
        bail!("node persistence unavailable: {error}");
    }
    let mut config = worker.configuration()?;
    let source = config
        .agent
        .agents_dir
        .clone()
        .or_else(opencoder_core::agent::agents_dir);
    let root = if legacy {
        worker
            .inner
            .layout
            .legacy_resources_dir(&assignment.index.id)?
    } else {
        worker
            .inner
            .layout
            .resources_dir(assignment.index.kind, &assignment.index.id)?
    };
    let new_snapshot = !root.exists();
    if new_snapshot {
        crate::resources::check_mount(config.agent.agents_dir.as_deref())?;
    }
    std::fs::create_dir_all(root.parent().unwrap())?;
    config.agent.agents_dir = crate::resources::pin(source.as_deref(), &root)?;
    let validated = (|| -> Result<()> {
        let prompt = assignment.request.input["prompt"].as_str().unwrap_or("");
        let needs_llm = match assignment.request.kind {
            ExecutionKind::Agent => !prompt.is_empty(),
            ExecutionKind::Dag => assignment
                .definition
                .as_ref()
                .and_then(|d| d.get("spec").unwrap_or(d).get("steps"))
                .and_then(Value::as_array)
                .is_some_and(|steps| steps.iter().any(|s| s["kind"]["type"] == "agent")),
            _ => true,
        };
        if needs_llm && worker.inner.client.is_none() {
            config.resolve_endpoint()?;
        }
        opencoder_core::agent::scope::with_root_sync(config.agent.agents_dir.clone(), || {
            let mut agents = vec![];
            match assignment.request.kind {
                ExecutionKind::Agent | ExecutionKind::Maintenance => agents.push(
                    assignment
                        .request
                        .target
                        .clone()
                        .unwrap_or_else(|| "act".into()),
                ),
                ExecutionKind::Team => {
                    let team: TeamDefinition = serde_json::from_value(
                        assignment
                            .definition
                            .clone()
                            .ok_or_else(|| anyhow::anyhow!("team definition missing"))?,
                    )?;
                    team.validate().map_err(anyhow::Error::msg)?;
                    agents.extend(team.members.into_iter().map(|m| m.agent));
                }
                ExecutionKind::Dag => {
                    let value = assignment
                        .definition
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("DAG definition missing"))?;
                    let spec: opencoder_dag::DagSpec =
                        serde_json::from_value(value.get("spec").unwrap_or(value).clone())?;
                    opencoder_dag::validate(&spec).map_err(|e| anyhow::anyhow!(e.join("; ")))?;
                    super::dag_preflight::validate(worker, &spec, legacy)?;
                    agents.extend(spec.steps.into_iter().filter_map(|s| match s.kind {
                        opencoder_dag::StepKind::Agent { agent, .. } => {
                            Some(agent.unwrap_or_else(|| "act".into()))
                        }
                        _ => None,
                    }));
                }
                ExecutionKind::Todos => {
                    let spec: opencoder_todos::WorkflowSpec = serde_json::from_value(
                        assignment
                            .definition
                            .clone()
                            .ok_or_else(|| anyhow::anyhow!("workflow definition missing"))?,
                    )?;
                    opencoder_todos::domain::validate_spec(&spec)?;
                    agents.extend(spec.todos.into_iter().map(|todo| todo.agent));
                }
                ExecutionKind::Project => {
                    let snapshot = assignment
                        .definition
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("project snapshot missing"))?;
                    let todo: opencoder_store::ProjectTodoRecord =
                        serde_json::from_value(snapshot["todo"].clone())?;
                    if Some(todo.id.as_str()) != assignment.request.target.as_deref() {
                        bail!("project snapshot target mismatch");
                    }
                    let _: Vec<opencoder_store::ProjectGoalRecord> =
                        serde_json::from_value(snapshot["goals"].clone())?;
                    let _: Vec<opencoder_store::ProjectMilestoneRecord> =
                        serde_json::from_value(snapshot["milestones"].clone())?;
                    agents.push(todo.agent);
                }
                _ => {}
            }
            for agent in agents {
                if opencoder_core::resolve_agent(&agent).is_none() {
                    bail!("agent {agent} unavailable");
                }
            }
            Ok::<_, anyhow::Error>(())
        })?;
        Ok(())
    })();
    if let Err(error) = validated {
        // Only accepted executions own immutable snapshots. A failed agent or
        // credential preflight must let a same-ID retry observe new resources.
        if new_snapshot {
            std::fs::remove_dir_all(&root).map_err(|cleanup| {
                anyhow::anyhow!("preflight failed: {error:#}; snapshot cleanup failed: {cleanup}")
            })?;
        }
        return Err(error);
    }
    Ok(config)
}

pub(crate) async fn start(
    worker: &Worker,
    id: &str,
    command: ExecutionCommand,
) -> Result<RpcReply> {
    let _gate = worker.inner.admission.lock().await;
    if worker.inner.active.lock().await.contains_key(id) {
        return Ok(RpcReply::error(409, "execution is running"));
    }
    if let Some(error) = worker.admission_error() {
        return Ok(RpcReply::error(503, error));
    }
    let (mut record, legacy) = {
        let journal = worker.inner.journal.lock().await;
        let Some(record) = journal.records.get(id).cloned() else {
            return Ok(RpcReply::error(404, "execution not found"));
        };
        (record, journal.uses_legacy(id))
    };
    if matches!(
        record.assignment.index.status,
        ExecutionStatus::Done | ExecutionStatus::Cancelled
    ) {
        return Ok(RpcReply::error(409, "execution is terminal"));
    }
    {
        let mut action = command.action;
        if matches!(action.as_str(), "plan" | "execute")
            && record.assignment.request.kind != ExecutionKind::Project
        {
            return Ok(RpcReply::error(
                400,
                "plan/execute requires project execution",
            ));
        }
        if action == "plan" {
            let Some(snapshot) = command.input.get("snapshot") else {
                return Ok(RpcReply::error(
                    400,
                    "plan requires a current project snapshot",
                ));
            };
            if snapshot["todo"]["id"].as_str() != record.assignment.request.target.as_deref() {
                return Ok(RpcReply::error(400, "project snapshot target mismatch"));
            }
            // Mutate only this private copy. Admission, capacity and preflight
            // must all pass before launch durably saves the next snapshot.
            record.result["next_snapshot"] = snapshot.clone();
        }
        if action == "resume" && record.assignment.request.kind == ExecutionKind::Project {
            let todo = record.assignment.request.target.as_deref().unwrap();
            let runs = worker
                .inner
                .state
                .project
                .require()?
                .projects
                .list_todo_runs(todo)
                .await?;
            action = if runs
                .first()
                .is_some_and(|r| r.kind == opencoder_store::ProjectTodoRunKind::Execute)
            {
                "execute"
            } else {
                "plan"
            }
            .into();
        }
        record.result["next_action"] = json!(action);
    }
    let permit = match worker.inner.slots.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return Ok(RpcReply::error(429, "node execution capacity exhausted")),
    };
    let mut effective = record.assignment.clone();
    if let Some(snapshot) = record.result.get("next_snapshot") {
        effective.definition = Some(snapshot.clone());
    }
    let config = match prepare(worker, &effective, legacy) {
        Ok(config) => config,
        Err(error) => {
            return Ok(RpcReply::error(
                400,
                format!("execution preflight: {error:#}"),
            ))
        }
    };
    match launch(worker.clone(), record, config, permit, true).await? {
        LaunchOutcome::Started => Ok(RpcReply::ok(json!({"id":id,"status":"running"}))),
        LaunchOutcome::Running => Ok(RpcReply::error(409, "execution is running")),
        LaunchOutcome::NotRunnable(status) => Ok(RpcReply::error(
            409,
            format!("execution cannot start from {}", status.as_str()),
        )),
    }
}

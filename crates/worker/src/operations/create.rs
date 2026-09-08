use crate::{journal::Record, lifecycle::Lifecycle, Worker};
use anyhow::{bail, Context, Result};
use opencoder_core::{fleet::*, Config};
use serde_json::{json, Value};

pub(super) use super::launch::{launch, launch_locked, LaunchOutcome};

/// Result-first override mirroring (same precedence as the workload's
/// brain_override): a re-execute command's stored resolution always
/// shadows any stale `brain` key left in request.input.
fn mirror_result_overrides(input: &mut Value, result: &Value) {
    if let Some(brain) = result.get("brain").filter(|v| !v.is_null()) {
        input["brain"] = brain.clone();
    }
}

pub(super) async fn create(worker: &Worker, mut assignment: Assignment) -> Result<RpcReply> {
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
        if assignment.request.kind == ExecutionKind::Project
            && assignment.request.input.get("run_id").is_none()
        {
            assignment.request.input["run_id"] =
                existing.assignment.request.input["run_id"].clone();
        }
        if existing.assignment.request != assignment.request {
            return Ok(RpcReply::error(
                409,
                "execution id already accepted with different input",
            ));
        }
        let mut body = json!(existing.assignment.index);
        if assignment.request.kind == ExecutionKind::Project {
            body["run_id"] = existing.assignment.request.input["run_id"].clone();
        }
        return Ok(RpcReply::ok(body));
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
    if assignment.request.kind == ExecutionKind::Project {
        super::project_admission::ensure_id(&mut assignment.request.input)?;
    }
    let config = match prepare(worker, &assignment, false) {
        Ok(config) => config,
        Err(error) => {
            return Ok(RpcReply::error(
                400,
                format!("execution preflight: {error:#}"),
            ))
        }
    };
    let project_run = if assignment.request.kind == ExecutionKind::Project {
        let action = assignment.request.input["action"]
            .as_str()
            .unwrap_or("plan");
        match super::project_admission::reserve(
            worker,
            &assignment,
            action,
            &assignment.request.input,
            &config,
        )
        .await
        {
            Ok(run) => Some(run),
            Err(error) => return Ok(super::project_admission::error_reply(error)),
        }
    } else {
        None
    };

    assignment.index.status = ExecutionStatus::Pending;
    let lifecycle = if assignment.index.kind == ExecutionKind::Todos {
        Lifecycle::accepted_todo()
    } else {
        Lifecycle::default()
    };
    let record = Record {
        assignment,
        result: project_run
            .as_ref()
            .map(|r| json!({"active_run_id":r.id,"next_run_id":r.id}))
            .unwrap_or(Value::Null),
        error: None,
        events: vec![],
        lifecycle,
    };
    worker.inner.journal.lock().await.save(record.clone())?;
    let _ = launch(worker.clone(), record.clone(), config, permit, false).await?;
    let mut body = json!(
        worker.inner.journal.lock().await.records[&record.assignment.index.id]
            .assignment
            .index
    );
    if let Some(run) = project_run {
        body["run_id"] = json!(run.id);
    }
    Ok(RpcReply::ok(body))
}

/// Executor-aware agent preflight for a project todo (pure). Agent todos
/// contribute their agent; team/dag contribute the agents named by their
/// INLINE spec (captain + members / agent steps) — a spec-less todo (or one
/// referencing an external resource via executor_ref) resolves lazily at
/// execute time, so nothing to preflight. Brain todos resolve on the
/// control plane: without a pre-resolution (input.brain) the node cannot
/// know the target and skips; with one, the resolved kind decides — agent
/// takes its ref (default `act`), team/dag reuse the todo's spec attempt.
fn project_preflight_agents(
    todo: &opencoder_store::ProjectTodoRecord,
    brain: Option<&Value>,
) -> Vec<String> {
    use opencoder_store::ProjectExecutorKind;
    match todo.executor_kind {
        ProjectExecutorKind::Agent => vec![todo.agent.clone()],
        ProjectExecutorKind::Team => {
            team_spec_agents(todo.executor_spec.as_deref()).unwrap_or_default()
        }
        ProjectExecutorKind::Dag => {
            dag_spec_agents(todo.executor_spec.as_deref()).unwrap_or_default()
        }
        ProjectExecutorKind::Brain => match brain.and_then(|b| b["kind"].as_str()) {
            Some("agent") => vec![brain
                .and_then(|b| b["ref"].as_str())
                .unwrap_or("act")
                .to_string()],
            Some("team") => team_spec_agents(todo.executor_spec.as_deref()).unwrap_or_default(),
            Some("dag") => dag_spec_agents(todo.executor_spec.as_deref()).unwrap_or_default(),
            _ => Vec::new(),
        },
    }
}

/// Captain + member `node_id`s off an inline team spec; None on a missing or
/// unparseable spec (lazy resolution, never a preflight failure).
fn team_spec_agents(spec: Option<&str>) -> Option<Vec<String>> {
    let spec: opencoder_project::executor::spec::TeamSpec = serde_json::from_str(spec?).ok()?;
    let mut agents = vec![spec.captain.node_id];
    agents.extend(spec.members.into_iter().map(|m| m.node_id));
    Some(agents)
}

/// Agent-step agents off an inline DagSpec (`agent.unwrap_or("act")`); None
/// on a missing or unparseable spec.
fn dag_spec_agents(spec: Option<&str>) -> Option<Vec<String>> {
    let spec: opencoder_dag::DagSpec = serde_json::from_str(spec?).ok()?;
    Some(
        spec.steps
            .into_iter()
            .filter_map(|step| match step.kind {
                opencoder_dag::StepKind::Agent { agent, .. } => {
                    Some(agent.unwrap_or_else(|| "act".into()))
                }
                _ => None,
            })
            .collect(),
    )
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
    let root = if assignment.request.kind == ExecutionKind::Project {
        let id = assignment.request.input["run_id"]
            .as_str()
            .context("project run id missing")?;
        opencoder_project::trace::archive::run_root(
            &worker.inner.data_dir.join("project-resources"),
            id,
        )?
    } else {
        root
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
                        opencoder_dag::decode_spec(value.get("spec").unwrap_or(value))
                            .map_err(|e| anyhow::anyhow!(e))?;
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
                    agents.extend(project_preflight_agents(
                        &todo,
                        assignment
                            .request
                            .input
                            .get("brain")
                            .filter(|v| !v.is_null()),
                    ));
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
    let mut command = command;
    if matches!(command.action.as_str(), "plan" | "execute") && id.starts_with("project-") {
        match super::project_admission::existing(worker, &id[8..], &command.action, &command.input)
            .await
        {
            Ok(Some(run)) => return Ok(super::project_admission::receipt(worker, id, &run)),
            Ok(None) => {}
            Err(error) => return Ok(RpcReply::error(409, error.to_string())),
        }
        super::project_admission::ensure_id(&mut command.input)?;
    }
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
        let mut action = command.action.clone();
        if matches!(action.as_str(), "plan" | "execute")
            && record.assignment.request.kind != ExecutionKind::Project
        {
            return Ok(RpcReply::error(
                400,
                "plan/execute requires project execution",
            ));
        }
        if matches!(action.as_str(), "plan" | "execute") && command.input.get("snapshot").is_some()
        {
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
        if action == "execute" {
            if let Some(brain) = command.input.get("brain").filter(|v| !v.is_null()) {
                // Control-plane brain pre-resolution rides the command on
                // the re-execute path; store it beside next_action so the
                // workload (and preflight below) can pick it up.
                record.result["brain"] = brain.clone();
            }
        }
        if record.assignment.request.kind == ExecutionKind::Project {
            super::project_admission::ensure_id(&mut command.input)?;
            record.result["next_run_id"] = command.input["run_id"].clone();
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
    if record.assignment.request.kind == ExecutionKind::Project {
        effective.request.input["run_id"] = command.input["run_id"].clone();
    }
    // Preflight reads the brain override off request.input; mirror the
    // command-carried resolution into this private copy (the durable
    // acceptance stays untouched) — result-first, so a stale input key
    // never shadows the re-execute resolution (same precedence as the
    // workload's brain_override).
    mirror_result_overrides(&mut effective.request.input, &record.result);
    let config = match prepare(worker, &effective, legacy) {
        Ok(config) => config,
        Err(error) => {
            return Ok(RpcReply::error(
                400,
                format!("execution preflight: {error:#}"),
            ))
        }
    };
    let project_run = if record.assignment.request.kind == ExecutionKind::Project {
        let action = record.result["next_action"].as_str().unwrap_or("plan");
        let run = match super::project_admission::reserve(
            worker,
            &effective,
            action,
            &command.input,
            &config,
        )
        .await
        {
            Ok(run) => run,
            Err(error) => return Ok(super::project_admission::error_reply(error)),
        };
        record.result["active_run_id"] = json!(run.id);
        Some(run)
    } else {
        None
    };
    match launch(worker.clone(), record, config, permit, true).await? {
        LaunchOutcome::Started => Ok(match project_run {
            Some(run) => super::project_admission::receipt(worker, id, &run),
            None => RpcReply::ok(json!({"id":id,"status":"running"})),
        }),
        LaunchOutcome::Running => Ok(RpcReply::error(409, "execution is running")),
        LaunchOutcome::NotRunnable(status) => Ok(RpcReply::error(
            409,
            format!("execution cannot start from {}", status.as_str()),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_store::{ProjectExecutorKind, ProjectTodoStatus};

    fn todo(kind: ProjectExecutorKind, spec: Option<&str>) -> opencoder_store::ProjectTodoRecord {
        opencoder_store::ProjectTodoRecord {
            id: "pt-1".into(),
            milestone_id: None,
            title: "t".into(),
            draft: "d".into(),
            plan_md: None,
            status: ProjectTodoStatus::Draft,
            agent: "act".into(),
            executor_kind: kind,
            executor_ref: None,
            executor_spec: spec.map(str::to_string),
            active_session_id: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn preflight_agents_follow_the_executor_kind() {
        // Agent: the todo's own agent.
        assert_eq!(
            project_preflight_agents(&todo(ProjectExecutorKind::Agent, None), None),
            vec!["act".to_string()]
        );
        // Team spec: captain + members node_ids; spec-less → lazy skip.
        let team = r#"{"name":"c","captain":{"node_id":"lead","name":"Lead"},"members":[{"node_id":"a1","name":"A"},{"node_id":"a2","name":"B"}]}"#;
        assert_eq!(
            project_preflight_agents(&todo(ProjectExecutorKind::Team, Some(team)), None),
            vec!["lead".to_string(), "a1".to_string(), "a2".to_string()]
        );
        assert!(project_preflight_agents(&todo(ProjectExecutorKind::Team, None), None).is_empty());
        assert!(
            project_preflight_agents(&todo(ProjectExecutorKind::Team, Some("{")), None).is_empty()
        );
        // Dag spec: agent steps with agent.unwrap_or("act"); wasm skipped.
        let dag = r#"{"name":"d","steps":[
            {"name":"w","kind":{"type":"wasm","command":"t.wasm"}},
            {"name":"x","kind":{"type":"agent","prompt":"p","agent":"explore"}},
            {"name":"y","kind":{"type":"agent","prompt":"p"}}]}"#;
        assert_eq!(
            project_preflight_agents(&todo(ProjectExecutorKind::Dag, Some(dag)), None),
            vec!["explore".to_string(), "act".to_string()]
        );
        assert!(project_preflight_agents(&todo(ProjectExecutorKind::Dag, None), None).is_empty());
    }

    #[test]
    fn brain_preflight_needs_a_resolution_and_follows_it() {
        let base = todo(ProjectExecutorKind::Brain, None);
        // Unresolved brain (no control-plane override) skips.
        assert!(project_preflight_agents(&base, None).is_empty());
        // Resolved agent takes ref (or act default).
        assert_eq!(
            project_preflight_agents(&base, Some(&json!({"kind":"agent","ref":"build"}))),
            vec!["build".to_string()]
        );
        assert_eq!(
            project_preflight_agents(&base, Some(&json!({"kind":"agent"}))),
            vec!["act".to_string()]
        );
        // Resolved team/dag reuse the todo's inline spec; a routes-table
        // spec (the brain default) does not parse as a team spec → skip.
        let team = r#"{"name":"c","captain":{"node_id":"lead","name":"Lead"}}"#;
        let routed = todo(ProjectExecutorKind::Brain, Some(team));
        assert_eq!(
            project_preflight_agents(&routed, Some(&json!({"kind":"team","ref":"crew"}))),
            vec!["lead".to_string()]
        );
        let routes = todo(ProjectExecutorKind::Brain, Some(r#"{"routes":[]}"#));
        assert!(project_preflight_agents(&routes, Some(&json!({"kind":"dag"}))).is_empty());
    }

    #[test]
    fn result_brain_mirrors_over_stale_input_brain() {
        // Result-first: a re-execute resolution shadows a pre-existing
        // (stale) input brain key — same precedence as brain_override.
        let mut input = json!({"brain": {"kind": "dag", "ref": "old"}});
        mirror_result_overrides(
            &mut input,
            &json!({"brain": {"kind": "agent", "ref": "act"}, "next_action": "execute"}),
        );
        assert_eq!(input["brain"]["ref"], "act");

        // Absent result brain leaves the input untouched.
        let mut input = json!({"brain": {"kind": "dag", "ref": "old"}});
        mirror_result_overrides(&mut input, &json!({"next_action": "execute"}));
        assert_eq!(input["brain"]["ref"], "old");

        // Null result brain likewise leaves the input untouched.
        let mut input = json!({"brain": {"kind": "dag", "ref": "old"}});
        mirror_result_overrides(&mut input, &json!({"brain": null}));
        assert_eq!(input["brain"]["ref"], "old");
    }
}

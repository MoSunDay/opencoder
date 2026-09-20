use crate::{journal::Record, lifecycle::Lifecycle, Worker};
use anyhow::{bail, Context, Result};
use opencoder_core::{fleet::*, Config};
use serde_json::{json, Value};

use super::admission::{preparation, replay::accepted_reply};
pub(super) use super::launch::{launch_locked, LaunchOutcome};

pub(super) async fn create(worker: &Worker, mut assignment: Assignment) -> Result<RpcReply> {
    if let Err(error) = assignment.request.validate() {
        return Ok(RpcReply::error(400, error));
    }
    if assignment.request.kind == ExecutionKind::System {
        return Ok(RpcReply::error(
            400,
            "system team execution is retired; use explicit node maintenance",
        ));
    }
    let input = &assignment.request.input;
    if (assignment.request.kind == ExecutionKind::Brain
        && !matches!(input["schema_version"].as_u64(), Some(2) | Some(3)))
        || (input.get("_brain").is_some() && input["_brain"]["schema_version"] != 2)
        || input.get("brain_receipt").is_some()
        || input.get("playbook_receipt").is_some()
    {
        return Ok(RpcReply::error(409, opencoder_brain::graph::MIGRATION));
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
    // A durable acceptance is a read-only replay. It must not queue behind
    // unrelated resource snapshots; otherwise a lost reply can never recover
    // under sustained admission load.
    if let Some(reply) = accepted_reply(worker, &mut assignment).await {
        return Ok(reply);
    }
    // Preparation precedes admission; execution lifecycle locks are taken
    // inside admission by dispatch. Sharing these gates creates a lock cycle
    // when a cold Create and its retry reach queue launch concurrently.
    let preparation = worker.preparation_gate(&assignment.index.id).await;
    let preparing = preparation.lock_owned().await;
    if let Some(reply) = accepted_reply(worker, &mut assignment).await {
        return Ok(reply);
    }
    let gate = worker.inner.admission.lock().await;
    // Another Create may have accepted this ID while we waited for the gate.
    if let Some(reply) = accepted_reply(worker, &mut assignment).await {
        return Ok(reply);
    }
    if let Some(error) = worker.admission_error() {
        return Ok(RpcReply::error(503, error));
    }
    if assignment.definition.is_none()
        && !matches!(
            assignment.request.kind,
            ExecutionKind::Agent | ExecutionKind::Maintenance | ExecutionKind::Operator
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
    assignment = match preparation::begin(worker, assignment)? {
        Ok(assignment) => assignment,
        Err(reply) => return Ok(reply),
    };
    // Cold filesystem reads are bounded per node and serialized per execution,
    // but never hold the gate needed by WASI admission, pause or cancellation.
    drop(gate);
    // Classify the frozen definition, including an interrupted preparation's
    // original snapshot, before reserving bounded resource-copy capacity.
    let resource_slot = if crate::resources::requires_agent_pool(&assignment) {
        Some(
            worker
                .inner
                .resource_preparations
                .clone()
                .acquire_owned()
                .await?,
        )
    } else {
        None
    };
    if let Some(error) = worker.admission_error() {
        return Ok(RpcReply::error(503, error));
    }
    let (prepared, (preparing, resource_slot)) =
        prepare_create(worker, &assignment, (preparing, resource_slot)).await?;
    let config = match prepared {
        Ok(config) => config,
        Err(error) => {
            preparation::reject_project(worker, &assignment)?;
            return Ok(RpcReply::error(
                400,
                format!("execution preflight: {error:#}"),
            ));
        }
    };
    // Fresh creations of operator executions materialize the per-execution
    // home/workspace (frozen config snapshot) BEFORE the record is enqueued,
    // so the first turn — and every restart resume — already resolves the
    // isolated paths. Deliberately NOT inside `prepare`: resume/command
    // paths re-run prepare for in-flight records, and materializing there
    // would flip a legacy execution's workdir mid-flight.
    if let Err(error) = preparation::blocking(|| {
        super::operator_env::materialize(
            &worker.inner.layout,
            assignment.request.kind,
            &assignment.index.id,
            &config,
        )
    }) {
        return Ok(RpcReply::error(
            400,
            format!("execution preflight: {error:#}"),
        ));
    }
    let _gate = worker.inner.admission.clone().lock_owned().await;
    if let Some(error) = worker.admission_error() {
        return Ok(RpcReply::error(503, error));
    }
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
            Err(error) => {
                let reply = super::project_admission::error_reply(error);
                if reply.status == 409 {
                    preparation::reject_project(worker, &assignment)?;
                }
                return Ok(reply);
            }
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
        annotations: if assignment.request.kind == ExecutionKind::Operator {
            json!({"operator_environment_version": 1})
        } else {
            Value::Null
        },
        queue: None,
        assignment,
        result: project_run
            .as_ref()
            .map(|r| json!({"active_run_id":r.id,"next_run_id":r.id}))
            .unwrap_or(Value::Null),
        error: None,
        events: vec![],
        lifecycle,
    };
    let record = super::queue::enqueue(worker, record, config, false).await?;
    preparation::finish(
        &worker
            .inner
            .layout
            .execution_dir(record.assignment.index.kind, &record.assignment.index.id)?,
    )?;
    drop(preparing);
    drop(resource_slot);
    let _gate = super::queue::dispatch_owned(worker, _gate).await?;
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

/// List agents used by an inline Project executor. Legacy brain modes fail admission.
fn project_preflight_agents(todo: &opencoder_store::ProjectTodoRecord) -> Vec<String> {
    use opencoder_store::ProjectExecutorKind;
    match todo.executor_kind {
        ProjectExecutorKind::Agent => vec![todo.agent.clone()],
        ProjectExecutorKind::Team => {
            team_spec_agents(todo.executor_spec.as_deref()).unwrap_or_default()
        }
        ProjectExecutorKind::Dag => {
            dag_spec_agents(todo.executor_spec.as_deref()).unwrap_or_default()
        }
        ProjectExecutorKind::Brain | ProjectExecutorKind::Playbook => Vec::new(),
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
            .filter_map(|step| match step.kind.executable() {
                opencoder_dag::StepKind::Agent { agent, .. } => {
                    Some(agent.clone().unwrap_or_else(|| "act".into()))
                }
                _ => None,
            })
            .collect(),
    )
}

pub(super) fn prepare(worker: &Worker, assignment: &Assignment, legacy: bool) -> Result<Config> {
    prepare_with_config(
        worker,
        assignment,
        legacy,
        worker.configuration()?,
        opencoder_core::agent::agents_dir(),
    )
}

async fn prepare_create(
    worker: &Worker,
    assignment: &Assignment,
    lease: preparation::Lease,
) -> Result<(Result<Config>, preparation::Lease)> {
    // Capture caller-scoped configuration before slow resource I/O crosses threads.
    let config = worker.configuration()?;
    let source = opencoder_core::agent::agents_dir();
    let worker = worker.clone();
    let assignment = assignment.clone();
    preparation::run(lease, move || {
        prepare_with_config(&worker, &assignment, false, config, source)
    })
    .await
}

pub(super) fn prepare_record(
    worker: &Worker,
    record: &Record,
    assignment: &Assignment,
    legacy: bool,
) -> Result<Config> {
    let config = crate::brain::workdir::execution_config(worker, record)?
        .map(Ok)
        .unwrap_or_else(|| worker.configuration())?;
    prepare_with_config(
        worker,
        assignment,
        legacy,
        config,
        opencoder_core::agent::agents_dir(),
    )
}

fn prepare_with_config(
    worker: &Worker,
    assignment: &Assignment,
    legacy: bool,
    mut config: Config,
    implicit_source: Option<std::path::PathBuf>,
) -> Result<Config> {
    let input = &assignment.request.input;
    anyhow::ensure!(
        !(assignment.request.kind == ExecutionKind::Brain
            && !matches!(input["schema_version"].as_u64(), Some(2) | Some(3)))
            && !(input.get("_brain").is_some() && input["_brain"]["schema_version"] != 2)
            && input.get("brain_receipt").is_none()
            && input.get("playbook_receipt").is_none(),
        "{}",
        opencoder_brain::graph::MIGRATION
    );
    if let Some(error) = worker.inner.persistence_error.lock().unwrap().as_ref() {
        bail!("node persistence unavailable: {error}");
    }
    if let Some(settings) = &assignment.runtime {
        config.agent.runtime = settings.as_ref().clone();
    }
    if let Some(settings) = &assignment.codex {
        settings.validate().map_err(anyhow::Error::msg)?;
        config.agent.codex = Some(settings.as_ref().clone());
    }
    if let Some(action) = assignment
        .request
        .input
        .get("_brain")
        .and_then(|b| b.get("action"))
    {
        if let Some(runtime) = action.get("runtime").filter(|v| !v.is_null()) {
            config.agent.runtime = serde_json::from_value(runtime.clone())?;
        }
        if let Some(codex) = action.get("codex").filter(|v| !v.is_null()) {
            config.agent.codex = Some(serde_json::from_value(codex.clone())?);
        }
    }
    let source = config
        .agent
        .agents_dir
        .clone()
        // An absent implicit pool permits built-in agents. An explicitly
        // configured source remains Some so pin rejects its disappearance.
        .or_else(|| implicit_source.filter(|path| path.exists()));
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
    let requires_agents = crate::resources::requires_agent_pool(assignment);
    if new_snapshot && requires_agents {
        crate::resources::check_mount(config.agent.agents_dir.as_deref())?;
    }
    std::fs::create_dir_all(root.parent().unwrap())?;
    let source = source.filter(|_| requires_agents);
    config.agent.agents_dir = crate::resources::pin(source.as_deref(), &root)?;
    let validated = (|| -> Result<()> {
        let prompt = assignment.request.input["prompt"].as_str().unwrap_or("");
        let needs_llm = match assignment.request.kind {
            ExecutionKind::Brain => true,
            ExecutionKind::Agent => !prompt.is_empty(),
            ExecutionKind::Dag => assignment
                .definition
                .as_ref()
                .and_then(|d| d.get("spec").unwrap_or(d).get("steps"))
                .and_then(Value::as_array)
                .is_some_and(|steps| {
                    steps.iter().any(|s| {
                        s["kind"]["type"] == "agent" || s["kind"]["template"]["type"] == "agent"
                    })
                }),
            _ => true,
        };
        opencoder_core::agent::scope::with_root_sync(config.agent.agents_dir.clone(), || {
            if let Some(pins) =
                assignment.request.input["_brain"]["action"]["agent_manifests"].as_object()
            {
                for (name, expected) in pins {
                    let actual = opencoder_core::brain::resources::agent_manifest(name)
                        .map_err(anyhow::Error::msg)?;
                    anyhow::ensure!(expected.as_str()==Some(actual.as_str()), "pinned agent resource mismatch for {name}; select a node with the required version");
                }
            }
            let mut agents = vec![];
            match assignment.request.kind {
                ExecutionKind::Brain => {
                    if input["schema_version"] == 2 {
                        opencoder_brain::execution::initialize(
                            &assignment.index.id,
                            serde_json::from_value(assignment.request.input.clone())?,
                            0,
                        )?;
                    } else if input["schema_version"] == 3 {
                        let request: opencoder_core::brain::BrainSchedulerRequest =
                            serde_json::from_value(input["scheduler_request"].clone())?;
                        opencoder_brain::scheduler::validate_request(&request)?;
                    }
                    if input["schema_version"] == 2 && worker.inner.client.is_none() {
                        crate::brain::activate::preflight()?;
                    }
                }
                ExecutionKind::Agent | ExecutionKind::Maintenance | ExecutionKind::Operator => {
                    agents.push(
                        assignment
                            .request
                            .target
                            .clone()
                            .unwrap_or_else(|| "act".into()),
                    )
                }
                ExecutionKind::Team => {
                    let mut team: TeamDefinition = serde_json::from_value(
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
                    super::dag_preflight::validate(worker, &config, &spec, legacy)?;
                    agents.extend(spec.steps.into_iter().filter_map(
                        |s| match s.kind.executable() {
                            opencoder_dag::StepKind::Agent { agent, .. } => {
                                Some(agent.clone().unwrap_or_else(|| "act".into()))
                            }
                            _ => None,
                        },
                    ));
                }
                ExecutionKind::Todos => {
                    agents.push("workflow".into());
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
                    anyhow::ensure!(
                        !matches!(
                            todo.executor_kind,
                            opencoder_store::ProjectExecutorKind::Brain
                                | opencoder_store::ProjectExecutorKind::Playbook
                        ),
                        "{}",
                        opencoder_brain::graph::MIGRATION
                    );
                    let assigned = project_preflight_agents(&todo);
                    // Validate the complete definition even while planning;
                    // credentials depend only on the agent executing this run.
                    for agent in &assigned {
                        if opencoder_core::resolve_agent(agent).is_none() {
                            bail!("agent {agent} unavailable");
                        }
                    }
                    if assignment.request.input["action"].as_str() == Some("execute") {
                        agents.extend(assigned);
                    } else {
                        agents.push("plan".into());
                    }
                }
                _ => {}
            }
            let selection: Option<opencoder_core::harness::Harness> = assignment
                .request
                .input
                .get("harness")
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()?;
            let envs: std::collections::BTreeMap<String, String> = assignment
                .request
                .input
                .get("envs")
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()?
                .unwrap_or_default();
            for (key, value) in &envs {
                opencoder_core::harness::validate_env(key, value).map_err(anyhow::Error::msg)?;
            }
            let mut native = agents.is_empty();
            for agent in agents {
                if opencoder_core::resolve_agent(&agent).is_none() {
                    bail!("agent {agent} unavailable");
                }
                // A `run_mode: agent` card routes every turn through a runc
                // sandbox round instead of the host session loop — reject
                // the execution at admission when that runtime is not
                // actually usable (fail closed, like the DAG preflights).
                if assignment.request.kind == ExecutionKind::Agent
                    && opencoder_core::agent::read_agent_meta(&agent)
                        .is_some_and(|meta| meta.run_mode == opencoder_core::agent::RunMode::Agent)
                {
                    crate::workloads::agent_runc::preflight(worker, &config, legacy)?;
                }
                let harness = if assignment.request.kind == ExecutionKind::Agent {
                    selection
                } else {
                    None
                }
                .unwrap_or_else(|| opencoder_core::harness::agent_harness(&agent));
                native |= harness == opencoder_core::harness::Harness::Opencoder;
                if harness != opencoder_core::harness::Harness::Codex {
                    continue;
                }
                let settings = opencoder_core::harness::agent_settings(&config, &agent)
                    .map_err(anyhow::Error::msg)?;
                if assignment.request.kind == ExecutionKind::Agent
                    && settings.is_some()
                    && (!envs.is_empty()
                        || assignment
                            .request
                            .input
                            .get("model")
                            .is_some_and(|v| !v.is_null()))
                {
                    bail!("Codex parameters are managed by Harness configuration; per-task model/env overrides are not allowed");
                }
                let mut effective_envs = settings.map(|s| s.envs.clone()).unwrap_or_default();
                effective_envs.extend(envs.clone());
                if assignment.request.kind == ExecutionKind::Dag
                    && config.dag.agent_sandbox == opencoder_core::config::AgentSandbox::Runc
                {
                    // dag_preflight validated the guest executable and node
                    // credentials; a host CLI is not used by this sandbox.
                    continue;
                }
                opencoder_session::harness::codex::configured_binary(
                    settings,
                    &effective_envs,
                    &crate::brain::workdir::node_workdir(worker),
                )?;
            }
            if needs_llm && native && worker.inner.client.is_none() {
                config.resolve_endpoint()?;
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
    let _gate = worker.inner.admission.clone().lock_owned().await;
    let mut command = command;
    if matches!(command.action.as_str(), "plan" | "execute") && id.starts_with("project-") {
        match super::project_admission::existing(worker, &id[8..], &command.action, &command.input)
            .await
        {
            Ok(Some(run)) => return Ok(super::project_admission::receipt(worker, id, &run).await),
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
    if record.assignment.index.status == ExecutionStatus::Pending {
        return Ok(RpcReply::error(409, "execution is already pending"));
    }
    if record
        .lifecycle
        .todo_reruns
        .values()
        .any(|c| c.phase == "stopping")
    {
        return Ok(RpcReply::error(
            409,
            "TODO rerun is still stopping the previous execution",
        ));
    }
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
    let mut effective = record.assignment.clone();
    if let Some(snapshot) = record.result.get("next_snapshot") {
        effective.definition = Some(snapshot.clone());
    }
    if record.assignment.request.kind == ExecutionKind::Project {
        effective.request.input["run_id"] = command.input["run_id"].clone();
        effective.request.input["action"] = record.result["next_action"].clone();
    }
    let config = match prepare_record(worker, &record, &effective, legacy) {
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
    super::queue::enqueue(worker, record, config, true).await?;
    let _gate = super::queue::dispatch_owned(worker, _gate).await?;
    let status = worker.inner.journal.lock().await.records[id]
        .assignment
        .index
        .status;
    let mut reply = match project_run {
        Some(run) => super::project_admission::receipt(worker, id, &run).await,
        None => RpcReply::ok(json!({"id":id})),
    };
    reply.body["status"] = json!(status);
    Ok(reply)
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
            project_preflight_agents(&todo(ProjectExecutorKind::Agent, None)),
            vec!["act".to_string()]
        );
        // Team spec: captain + members node_ids; spec-less → lazy skip.
        let team = r#"{"name":"c","captain":{"node_id":"lead","name":"Lead"},"members":[{"node_id":"a1","name":"A"},{"node_id":"a2","name":"B"}]}"#;
        assert_eq!(
            project_preflight_agents(&todo(ProjectExecutorKind::Team, Some(team))),
            vec!["lead".to_string(), "a1".to_string(), "a2".to_string()]
        );
        assert!(project_preflight_agents(&todo(ProjectExecutorKind::Team, None)).is_empty());
        assert!(project_preflight_agents(&todo(ProjectExecutorKind::Team, Some("{"))).is_empty());
        // Dag spec: agent steps with agent.unwrap_or("act"); wasm skipped.
        let dag = r#"{"name":"d","steps":[
            {"name":"w","kind":{"type":"wasm","command":"t.wasm"}},
            {"name":"x","kind":{"type":"agent","prompt":"p","agent":"explore"}},
            {"name":"y","kind":{"type":"agent","prompt":"p"}}]}"#;
        assert_eq!(
            project_preflight_agents(&todo(ProjectExecutorKind::Dag, Some(dag))),
            vec!["explore".to_string(), "act".to_string()]
        );
        assert!(project_preflight_agents(&todo(ProjectExecutorKind::Dag, None)).is_empty());
    }
}

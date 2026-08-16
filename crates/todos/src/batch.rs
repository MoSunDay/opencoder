use anyhow::{Context, Result};
use tokio::task::JoinSet;

use crate::{execution, parent, runner::Runtime, transitions, types::*};

pub async fn execute(
    runtime: &Runtime,
    spec: &WorkflowSpec,
    state: &mut WorkflowState,
    requests: Vec<DispatchTodo>,
    reason: String,
) -> Result<()> {
    if requests.is_empty() {
        anyhow::bail!("parent dispatch cannot be empty");
    }
    let assignments = assignments(state, &requests)?;
    prepare_sessions(runtime, spec, &assignments).await?;
    *state = transitions::dispatch(spec, state.clone(), &assignments)?;
    let dispatched_ids: Vec<String> = requests.iter().map(|r| r.todo_id.clone()).collect();
    let dispatch_reason = reason.clone();
    runtime
        .commit(
            spec,
            state,
            "todos_dispatched",
            serde_json::json!({"todos":requests,"reason":reason}),
        )
        .await?;
    tracing::info!(
        workflow_id = %state.workflow_id,
        todo_ids = ?dispatched_ids,
        reason = %dispatch_reason,
        "todos dispatched"
    );

    let results = run_assignments(runtime, spec, state, assignments).await?;
    let mut fatal: Option<anyhow::Error> = None;
    for (todo_id, result) in results {
        if let Err(error) = apply_result(runtime, spec, state, &todo_id, result).await {
            if fatal.is_none() {
                fatal = Some(error);
            }
        }
    }
    match fatal {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

async fn apply_result(
    runtime: &Runtime,
    spec: &WorkflowSpec,
    state: &mut WorkflowState,
    todo_id: &str,
    result: Result<execution::TodoExecution>,
) -> Result<()> {
    let execution = match result {
        Ok(execution) => execution,
        Err(error) => {
            let interrupted = runtime.cancel.is_cancelled();
            let reason = format!("{error:#}");
            *state = transitions::execution_failed(
                spec,
                state.clone(),
                todo_id,
                reason.clone(),
                interrupted,
            )?;
            runtime
                .commit(
                    spec,
                    state,
                    "todo_execution_failed",
                    serde_json::json!({"todo_id":todo_id,"reason":reason,"interrupted":interrupted}),
                )
                .await?;
            return Ok(());
        }
    };
    *state = transitions::candidate(state.clone(), todo_id, execution.candidate)?;
    runtime
        .commit(
            spec,
            state,
            "todo_candidate_ready",
            serde_json::json!({"todo_id":todo_id,"gate":execution.gate}),
        )
        .await?;
    if state
        .todos
        .get(todo_id)
        .map(|todo| todo.status)
        != Some(TodoStatus::CandidateReady)
    {
        return Ok(());
    }
    *state = transitions::accepting(state.clone(), todo_id)?;
    runtime
        .commit(
            spec,
            state,
            "todo_acceptance_started",
            serde_json::json!({"todo_id":todo_id}),
        )
        .await?;
    let decision = parent::accept(
        &runtime.parent_runtime(),
        spec,
        state,
        todo_id,
        &execution.gate,
    )
    .await?;
    apply_acceptance(runtime, spec, state, todo_id, execution.gate, decision).await
}

fn assignments(
    state: &WorkflowState,
    requests: &[DispatchTodo],
) -> Result<Vec<(DispatchTodo, String)>> {
    requests
        .iter()
        .map(|request| {
            let session_id = if request.context_mode == ContextMode::Resume {
                state
                    .todos
                    .get(&request.todo_id)
                    .with_context(|| format!("state missing TODO {}", request.todo_id))?
                    .active_session_id
                    .clone()
                    .context("resume session missing")?
            } else {
                format!("todo-{}", ulid::Ulid::new())
            };
            Ok((request.clone(), session_id))
        })
        .collect()
}

async fn prepare_sessions(
    runtime: &Runtime,
    spec: &WorkflowSpec,
    assignments: &[(DispatchTodo, String)],
) -> Result<()> {
    for (request, session_id) in assignments {
        if request.context_mode != ContextMode::Resume {
            let todo = spec
                .todos
                .iter()
                .find(|todo| todo.id == request.todo_id)
                .expect("validated TODO");
            execution::prepare_session(&runtime.store, spec, todo, session_id, &runtime.config)
                .await?;
        }
    }
    Ok(())
}

async fn run_assignments(
    runtime: &Runtime,
    spec: &WorkflowSpec,
    state: &WorkflowState,
    assignments: Vec<(DispatchTodo, String)>,
) -> Result<Vec<(String, Result<execution::TodoExecution>)>> {
    let mut jobs = JoinSet::new();
    for (request, assigned_id) in assignments {
        let todo = spec
            .todos
            .iter()
            .find(|todo| todo.id == request.todo_id)
            .expect("validated TODO")
            .clone();
        let todo_id = todo.id.clone();
        let runtime = runtime.clone();
        let spec = spec.clone();
        let workflow_id = state.workflow_id.clone();
        let snapshot = state.clone();
        let token = runtime.cancel.child_token();
        jobs.spawn(async move {
            let poll = super::runner::poll_interrupt(
                runtime.store.clone(),
                workflow_id,
                snapshot.generation,
                token.clone(),
            );
            tokio::pin!(poll);
            let execute = execution::execute(
                runtime.store.clone(),
                runtime.client.clone(),
                runtime.config.clone(),
                &runtime.workdir,
                &spec,
                &snapshot,
                &todo,
                request.context_mode,
                assigned_id,
                token.clone(),
            );
            tokio::pin!(execute);
            let result = tokio::select! {
                result = &mut execute => result,
                _ = &mut poll => {
                    token.cancel();
                    execute.await
                }
            };
            (todo_id, result)
        });
    }
    let mut results = Vec::new();
    while let Some(joined) = jobs.join_next().await {
        results.push(joined.context("TODO execution task panicked")?);
    }
    Ok(results)
}

async fn apply_acceptance(
    runtime: &Runtime,
    spec: &WorkflowSpec,
    state: &mut WorkflowState,
    todo_id: &str,
    gate: serde_json::Value,
    decision: AcceptanceDecision,
) -> Result<()> {
    match decision {
        AcceptanceDecision::Accept {
            reason,
            mark_milestone,
        } => {
            if gate["ok"] != true {
                anyhow::bail!("parent accepted TODO {todo_id} despite failed tool gate");
            }
            *state = transitions::accepted(state.clone(), todo_id, mark_milestone)?;
            runtime.commit(spec, state, "todo_accepted", serde_json::json!({"todo_id":todo_id,"reason":reason,"mark_milestone":mark_milestone})).await?;
            tracing::info!(
                workflow_id = %state.workflow_id,
                todo_id = %todo_id,
                mark_milestone,
                "todo accepted"
            );
        }
        AcceptanceDecision::Revise {
            reason,
            context_mode,
        } => {
            *state =
                transitions::revise(spec, state.clone(), todo_id, reason.clone(), context_mode)?;
            runtime.commit(spec, state, "todo_revision_requested", serde_json::json!({"todo_id":todo_id,"reason":reason,"context_mode":context_mode})).await?;
            tracing::info!(
                workflow_id = %state.workflow_id,
                todo_id = %todo_id,
                "todo revision requested"
            );
        }
        AcceptanceDecision::Fail { reason } => {
            if let Some(todo) = state.todos.get_mut(todo_id) {
                todo.status = TodoStatus::Failed;
                todo.last_error = Some(reason.clone());
            }
            state.generation += 1;
            runtime
                .commit(
                    spec,
                    state,
                    "todo_failed",
                    serde_json::json!({"todo_id":todo_id,"reason":reason}),
                )
                .await?;
            tracing::info!(
                workflow_id = %state.workflow_id,
                todo_id = %todo_id,
                "todo failed"
            );
        }
        AcceptanceDecision::Rewind {
            milestone_todo_id,
            reason,
        } => {
            *state = transitions::rewind(spec, state.clone(), &milestone_todo_id, reason.clone())?;
            runtime
                .commit(
                    spec,
                    state,
                    "workflow_rewound",
                    serde_json::json!({"milestone_todo_id":milestone_todo_id,"reason":reason}),
                )
                .await?;
        }
    }
    Ok(())
}

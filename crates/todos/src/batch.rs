use std::sync::Arc;

use anyhow::{Context, Result};
use opencoder_store::Store;
use tokio::task::JoinSet;

use crate::{execution, parent, persistence, runner::Runtime, transitions, types::*};

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
            let reason = format!("{error:#}");
            // Mirror the late-Ok guard below: a sibling acceptance can rewind
            // a milestone and invalidate this TODO while its (failing)
            // execution is still in flight. A late Err must be discarded the
            // same way — routing it into execution_failed would trip the
            // Running guard's `require` and suspend the whole workflow over a
            // result nobody is waiting for anymore.
            let current = state.todos.get(todo_id).map(|todo| todo.status);
            if current != Some(TodoStatus::Running) {
                tracing::info!(
                    workflow_id = %state.workflow_id,
                    todo_id = todo_id,
                    status = ?current,
                    "discarding execution failure for TODO that is no longer running"
                );
                return Ok(());
            }
            // An external interrupt (`runner::interrupt` from another process)
            // persists Suspended directly to the store. The local cancel token
            // is only flipped by `poll_interrupt` on its 250ms cadence, so a
            // token-only check has a window where the external interrupt is
            // already durable but still looks like a plain execution failure —
            // which would mark the todo Failed once attempts are exhausted
            // and clobber the externally written Suspended state. Re-check the
            // store before taking the local failure path.
            if externally_suspended(&runtime.store, state).await {
                tracing::info!(
                    workflow_id = %state.workflow_id,
                    todo_id = todo_id,
                    reason = %reason,
                    "todo execution error superseded by externally suspended workflow; keeping persisted state"
                );
                return Ok(());
            }
            let interrupted = runtime.cancel.is_cancelled();
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
            tracing::info!(
                workflow_id = %state.workflow_id,
                todo_id = %todo_id,
                interrupted,
                "todo execution failed"
            );
            return Ok(());
        }
    };
    // Bug #16a: the TODO's status may have moved on while it executed — a
    // sibling acceptance can rewind a milestone and invalidate it, and an
    // external interrupt can have flipped the persisted workflow to
    // Suspended. The execution result itself is not wrong, so discard it
    // instead of failing the whole round over `candidate`'s Running guard
    // (which would suspend the workflow or clobber the external verdict).
    let current = state.todos.get(todo_id).map(|todo| todo.status);
    if current != Some(TodoStatus::Running) {
        tracing::info!(
            workflow_id = %state.workflow_id,
            todo_id = todo_id,
            status = ?current,
            "discarding execution result for TODO that is no longer running"
        );
        return Ok(());
    }
    if externally_suspended(&runtime.store, state).await {
        tracing::info!(
            workflow_id = %state.workflow_id,
            todo_id = todo_id,
            "todo execution result superseded by externally suspended workflow; keeping persisted state"
        );
        return Ok(());
    }
    *state = transitions::candidate(spec, state.clone(), todo_id, execution.candidate)?;
    runtime
        .commit(
            spec,
            state,
            "todo_candidate_ready",
            serde_json::json!({"todo_id":todo_id,"gate":execution.gate}),
        )
        .await?;
    if state.todos.get(todo_id).map(|todo| todo.status) != Some(TodoStatus::CandidateReady) {
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
    // Bug #16b/M6 symmetry: only dispatch decisions used to get correctable
    // re-asks. An invalid acceptance decision (accepting a failed gate,
    // rewinding to a non-milestone, revising a terminal todo) was a one-shot
    // mistake that suspended the whole workflow. Dry-run every acceptance
    // decision against a throwaway state clone and re-ask with a correction,
    // bounded like the dispatch side.
    let parent_runtime = runtime.parent_runtime();
    let mut correction: Option<String> = None;
    let mut retries_left = ACCEPTANCE_CORRECTION_RETRIES;
    let decision = loop {
        let decision = parent::accept(
            &parent_runtime,
            spec,
            state,
            todo_id,
            &execution.gate,
            correction.as_deref(),
        )
        .await?;
        match validate_acceptance(spec, state, todo_id, &execution.gate, &decision) {
            Ok(()) => break decision,
            Err(error) if retries_left > 0 => {
                retries_left -= 1;
                let reason = format!("{error:#}");
                tracing::info!(
                    workflow_id = %state.workflow_id,
                    todo_id = todo_id,
                    error = %reason,
                    retries_left,
                    "parent acceptance rejected; re-asking with correction"
                );
                correction = Some(format!(
                    "your previous acceptance decision was rejected: {reason}. Honor the required tool gates and the allowed operations, or choose fail."
                ));
            }
            Err(error) => {
                return Err(
                    error.context("parent acceptance stayed invalid after correction retries")
                );
            }
        }
    };
    apply_acceptance(runtime, spec, state, todo_id, execution.gate, decision).await
}

/// True when an external writer (e.g. `runner::interrupt` from another
/// process) already persisted a Suspended verdict for this workflow. While a
/// batch is running or applying results the local in-memory state can lag
/// behind such a write (the local token is only flipped by `poll_interrupt`'s
/// 250ms poll), so both execution errors and successful results must not be
/// mapped onto local transitions that would overwrite it.
///
/// A bare generation bump without Suspended is deliberately NOT treated as an
/// interrupt: that is the external-change conflict case, which must keep the
/// local failure path so the runtime still observes the conflict and persists
/// a suspension instead of silently continuing.
async fn externally_suspended(store: &Arc<dyn Store>, state: &WorkflowState) -> bool {
    match persistence::load(store, &state.workflow_id).await {
        Ok(Some((_, latest))) => latest.status == WorkflowStatus::Suspended,
        Ok(None) => false,
        Err(error) => {
            tracing::warn!(
                workflow_id = %state.workflow_id,
                error = %error,
                "could not re-check store for external workflow suspension; treating as local"
            );
            false
        }
    }
}

/// Maximum acceptance-correction re-asks around `parent::accept`: 1 initial
/// decision plus 2 corrected re-decisions before an invalid acceptance bails.
const ACCEPTANCE_CORRECTION_RETRIES: u32 = 2;

/// Pure validation of one acceptance decision: dry-runs the same transitions
/// `apply_acceptance` performs on a cloned state that is then dropped, so an
/// invalid model decision can be corrected with a re-ask instead of
/// suspending the workflow. Mirrors `validate_request` on the dispatch side.
pub(crate) fn validate_acceptance(
    spec: &WorkflowSpec,
    state: &WorkflowState,
    todo_id: &str,
    gate: &serde_json::Value,
    decision: &AcceptanceDecision,
) -> Result<()> {
    match decision {
        AcceptanceDecision::Accept { .. } => {
            if gate["ok"] != true {
                anyhow::bail!("cannot accept TODO {todo_id}: required tool gate failed");
            }
            Ok(())
        }
        AcceptanceDecision::Revise { context_mode, .. } => {
            transitions::revise(spec, state.clone(), todo_id, String::new(), *context_mode)
                .map(|_| ())
        }
        AcceptanceDecision::Rewind {
            milestone_todo_id, ..
        } => transitions::rewind(spec, state.clone(), milestone_todo_id, String::new()).map(|_| ()),
        AcceptanceDecision::Fail { .. } => Ok(()),
    }
}

/// Pure validation of one parent scheduling decision: dispatch reuses
/// `validate_request` (the exact `transitions::dispatch` rules); every other
/// operation dry-runs its transition on a cloned state that is then dropped.
/// `runner::drive_inner` uses this to correct-and-re-ask invalid decisions of
/// ANY kind, not just dispatches.
pub(crate) fn validate_decision(
    spec: &WorkflowSpec,
    state: &WorkflowState,
    decision: &ParentDecision,
) -> Result<()> {
    match decision {
        ParentDecision::Dispatch { todos, .. } => validate_request(spec, state, todos),
        ParentDecision::MarkMilestone { todo_id, .. } => {
            // Re-marking an existing milestone is idempotent (mirrors
            // accepted(mark_milestone=true)'s silent set-insert), so only the
            // Passed requirement is validated here.
            match state.todos.get(todo_id).map(|todo| todo.status) {
                Some(TodoStatus::Passed) => Ok(()),
                Some(_) => anyhow::bail!("parent tried to mark non-passed milestone {todo_id}"),
                None => anyhow::bail!("unknown TODO {todo_id}"),
            }
        }
        ParentDecision::Rewind {
            milestone_todo_id, ..
        } => transitions::rewind(spec, state.clone(), milestone_todo_id, String::new()).map(|_| ()),
        ParentDecision::Complete { .. } => {
            transitions::terminal(state.clone(), WorkflowStatus::Completed, String::new())
                .map(|_| ())
        }
        ParentDecision::Fail { .. } | ParentDecision::Suspend { .. } => Ok(()),
    }
}

/// Dry-run the parent's dispatch decision against the exact validation
/// `transitions::dispatch` applies inside `execute` — no sessions are
/// created and nothing is mutated. `runner::drive_inner` uses this to
/// reject a malformed model decision and re-ask the parent with a
/// correction prompt before any durable state changes.
pub(crate) fn validate_request(
    spec: &WorkflowSpec,
    state: &WorkflowState,
    requests: &[DispatchTodo],
) -> Result<()> {
    if requests.is_empty() {
        anyhow::bail!("parent dispatch cannot be empty");
    }
    let assignments = assignments(state, requests)?;
    transitions::validate_dispatch(spec, state, requests, &assignments)
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
            tracing::info!(
                workflow_id = %state.workflow_id,
                milestone_todo_id = %milestone_todo_id,
                "workflow rewound to milestone"
            );
        }
    }
    Ok(())
}

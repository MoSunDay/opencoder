use anyhow::{bail, Result};

use crate::{domain, types::*};

pub fn reconcile_interrupted(mut state: WorkflowState) -> WorkflowState {
    let active: Vec<String> = state.active_todo_ids.iter().cloned().collect();
    for id in active {
        if let Some(todo) = state.todos.get_mut(&id) {
            todo.status = TodoStatus::Interrupted;
            todo.last_error = Some("runtime stopped before TODO acceptance".into());
        }
    }
    state.active_todo_ids.clear();
    if state.status == WorkflowStatus::Running {
        state.status = WorkflowStatus::Suspended;
    }
    bump(&mut state);
    state
}

pub fn dispatch(
    spec: &WorkflowSpec,
    mut state: WorkflowState,
    requests: &[(DispatchTodo, String)],
) -> Result<WorkflowState> {
    let runnable = domain::runnable(spec, &state);
    let mut seen = std::collections::HashSet::new();
    for (request, session_id) in requests {
        if !seen.insert(request.todo_id.as_str()) {
            bail!("TODO {} was dispatched more than once", request.todo_id);
        }
        if !runnable.contains(&request.todo_id) {
            bail!("TODO {} is not runnable", request.todo_id);
        }
        let spec_todo = spec
            .todos
            .iter()
            .find(|todo| todo.id == request.todo_id)
            .expect("validated spec");
        let todo = state
            .todos
            .get_mut(&request.todo_id)
            .expect("validated state");
        if todo.attempt >= spec_todo.max_attempts {
            bail!("TODO {} exhausted max_attempts", request.todo_id);
        }
        match request.context_mode {
            ContextMode::New if todo.attempt != 0 => {
                bail!(
                    "TODO {} can use new only on its first attempt",
                    request.todo_id
                )
            }
            ContextMode::Resume if todo.active_session_id.is_none() => {
                bail!("TODO {} has no session to resume", request.todo_id)
            }
            _ => {}
        }
        if let Some(required) = todo.next_context_mode {
            if request.context_mode != required {
                bail!(
                    "TODO {} must use context_mode {required:?}",
                    request.todo_id
                );
            }
        }
        todo.attempt += 1;
        if request.context_mode != ContextMode::Resume {
            todo.active_session_id = Some(session_id.clone());
            todo.session_history.push(session_id.clone());
        }
        todo.status = TodoStatus::Running;
        todo.candidate = None;
        todo.last_error = None;
        todo.next_context_mode = None;
        state.active_todo_ids.insert(request.todo_id.clone());
    }
    state.status = WorkflowStatus::Running;
    bump(&mut state);
    Ok(state)
}

pub fn started(mut state: WorkflowState, todo_id: &str) -> Result<WorkflowState> {
    let todo = item(&mut state, todo_id)?;
    require(todo.status, &[TodoStatus::Dispatching])?;
    todo.status = TodoStatus::Running;
    bump(&mut state);
    Ok(state)
}

pub fn candidate(
    mut state: WorkflowState,
    todo_id: &str,
    value: Candidate,
) -> Result<WorkflowState> {
    let todo = item(&mut state, todo_id)?;
    require(todo.status, &[TodoStatus::Running])?;
    todo.status = match value.status {
        CandidateStatus::Candidate => TodoStatus::CandidateReady,
        CandidateStatus::Blocked => TodoStatus::NeedsRevision,
        CandidateStatus::Interrupted => TodoStatus::Interrupted,
    };
    if todo.status != TodoStatus::CandidateReady {
        todo.last_error = Some(value.summary.clone());
    }
    todo.candidate = Some(value);
    state.active_todo_ids.remove(todo_id);
    bump(&mut state);
    Ok(state)
}

pub fn accepting(mut state: WorkflowState, todo_id: &str) -> Result<WorkflowState> {
    let todo = item(&mut state, todo_id)?;
    require(todo.status, &[TodoStatus::CandidateReady])?;
    todo.status = TodoStatus::Accepting;
    bump(&mut state);
    Ok(state)
}

pub fn accepted(mut state: WorkflowState, todo_id: &str, milestone: bool) -> Result<WorkflowState> {
    let generation = state.generation + 1;
    let todo = item(&mut state, todo_id)?;
    require(todo.status, &[TodoStatus::Accepting])?;
    todo.status = TodoStatus::Passed;
    todo.accepted_generation = Some(generation);
    todo.last_error = None;
    if milestone {
        state.milestones.insert(todo_id.to_string());
    }
    bump(&mut state);
    Ok(state)
}

pub fn revise(
    spec: &WorkflowSpec,
    mut state: WorkflowState,
    todo_id: &str,
    reason: String,
    context_mode: ContextMode,
) -> Result<WorkflowState> {
    let spec_todo = spec
        .todos
        .iter()
        .find(|todo| todo.id == todo_id)
        .ok_or_else(|| anyhow::anyhow!("unknown TODO {todo_id}"))?;
    let todo = item(&mut state, todo_id)?;
    require(
        todo.status,
        &[TodoStatus::Accepting, TodoStatus::CandidateReady],
    )?;
    todo.last_error = Some(reason);
    todo.next_context_mode = Some(context_mode);
    todo.status = if todo.attempt >= spec_todo.max_attempts {
        TodoStatus::Failed
    } else {
        TodoStatus::NeedsRevision
    };
    bump(&mut state);
    Ok(state)
}

pub fn rewind(
    spec: &WorkflowSpec,
    mut state: WorkflowState,
    milestone_id: &str,
    reason: String,
) -> Result<WorkflowState> {
    if !state.milestones.contains(milestone_id) {
        bail!("unknown milestone TODO {milestone_id}");
    }
    state.world_epoch += 1;
    let affected = domain::descendants(spec, milestone_id);
    for id in affected {
        if let Some(todo) = state.todos.get_mut(&id) {
            todo.status = TodoStatus::Invalidated;
            todo.accepted_generation = None;
            todo.last_error = Some(reason.clone());
        }
        state.active_todo_ids.remove(&id);
    }
    let milestone = item(&mut state, milestone_id)?;
    milestone.status = TodoStatus::Recovering;
    milestone.accepted_generation = None;
    milestone.last_error = Some(reason.clone());
    state.incidents.push(serde_json::json!({
        "milestone_todo_id": milestone_id,
        "reason": reason,
        "world_epoch": state.world_epoch
    }));
    bump(&mut state);
    Ok(state)
}

pub fn terminal(
    mut state: WorkflowState,
    status: WorkflowStatus,
    reason: String,
) -> Result<WorkflowState> {
    match status {
        WorkflowStatus::Completed => {
            if state
                .todos
                .values()
                .any(|todo| todo.status != TodoStatus::Passed)
            {
                bail!("cannot complete with unaccepted TODOs");
            }
        }
        WorkflowStatus::Suspended => {
            let active: Vec<String> = state.active_todo_ids.iter().cloned().collect();
            for id in active {
                if let Some(todo) = state.todos.get_mut(&id) {
                    todo.status = TodoStatus::Interrupted;
                    todo.last_error = Some(reason.clone());
                }
            }
        }
        WorkflowStatus::Failed => {}
        _ => bail!("invalid terminal status"),
    }
    state.status = status;
    state.terminal_reason = Some(reason);
    state.active_todo_ids.clear();
    bump(&mut state);
    Ok(state)
}

fn item<'a>(state: &'a mut WorkflowState, id: &str) -> Result<&'a mut TodoState> {
    state
        .todos
        .get_mut(id)
        .ok_or_else(|| anyhow::anyhow!("unknown TODO {id}"))
}

fn require(actual: TodoStatus, allowed: &[TodoStatus]) -> Result<()> {
    if !allowed.contains(&actual) {
        bail!("invalid TODO transition from {actual:?}");
    }
    Ok(())
}

fn bump(state: &mut WorkflowState) {
    state.generation += 1;
}

use anyhow::{bail, Result};

use crate::{domain, types::*};

pub fn reconcile_interrupted(mut state: WorkflowState) -> WorkflowState {
    for (_, todo) in state.todos.iter_mut() {
        if matches!(
            todo.status,
            TodoStatus::Running | TodoStatus::CandidateReady | TodoStatus::Accepting
        ) {
            todo.status = TodoStatus::Interrupted;
            todo.last_error = Some("runtime stopped before TODO acceptance".into());
            todo.candidate = None;
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
    let runnable = domain::runnable(spec, &state)?;
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

pub fn execution_failed(
    spec: &WorkflowSpec,
    mut state: WorkflowState,
    todo_id: &str,
    reason: String,
    interrupted: bool,
) -> Result<WorkflowState> {
    let spec_todo = spec
        .todos
        .iter()
        .find(|todo| todo.id == todo_id)
        .ok_or_else(|| anyhow::anyhow!("unknown TODO {todo_id}"))?;
    let todo = item(&mut state, todo_id)?;
    require(
        todo.status,
        &[
            TodoStatus::Running,
            TodoStatus::CandidateReady,
            TodoStatus::Accepting,
        ],
    )?;
    todo.last_error = Some(reason);
    todo.status = if interrupted {
        TodoStatus::Interrupted
    } else if todo.attempt >= spec_todo.max_attempts {
        TodoStatus::Failed
    } else {
        TodoStatus::NeedsRevision
    };
    state.active_todo_ids.remove(todo_id);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_todo(id: &str, max_attempts: u32) -> TodoSpec {
        TodoSpec {
            id: id.into(),
            title: format!("title {id}"),
            requirement_background: "background".into(),
            instructions: "instructions".into(),
            depends_on: Vec::new(),
            agent: "act".into(),
            max_attempts,
            acceptance: AcceptanceSpec {
                criteria: "done".into(),
                required_tool_calls: Vec::new(),
            },
            metadata: serde_json::Value::Null,
        }
    }

    fn spec_with(ids: &[&str], max_attempts: u32) -> WorkflowSpec {
        WorkflowSpec {
            schema_version: 1,
            id: "wf".into(),
            name: "wf".into(),
            objective: "objective".into(),
            constraints: Vec::new(),
            todos: ids.iter().map(|id| spec_todo(id, max_attempts)).collect(),
            metadata: serde_json::Value::Null,
        }
    }

    fn state(spec: &WorkflowSpec) -> WorkflowState {
        domain::initial_state(spec, "wf-run".into(), "parent".into())
    }

    fn candidate_value() -> Candidate {
        Candidate {
            status: CandidateStatus::Candidate,
            summary: "summary".into(),
            result: Some("ok".into()),
            verification: "verified".into(),
            evidence_refs: Vec::new(),
            recovery_context: RecoveryContext::default(),
        }
    }

    #[test]
    fn reconcile_rolls_back_candidate_ready_todo() {
        let workflow = spec_with(&["t1"], 3);
        let mut workflow_state = state(&workflow);
        let todo = workflow_state.todos.get_mut("t1").unwrap();
        todo.status = TodoStatus::CandidateReady;
        todo.candidate = Some(candidate_value());
        let (epoch, incidents, generation) = (
            workflow_state.world_epoch,
            workflow_state.incidents.clone(),
            workflow_state.generation,
        );

        let reconciled = reconcile_interrupted(workflow_state);

        let todo = &reconciled.todos["t1"];
        assert_eq!(todo.status, TodoStatus::Interrupted);
        assert_eq!(todo.candidate, None);
        assert_eq!(
            todo.last_error.as_deref(),
            Some("runtime stopped before TODO acceptance")
        );
        assert!(!reconciled.active_todo_ids.contains("t1"));
        assert_eq!(reconciled.world_epoch, epoch);
        assert_eq!(reconciled.incidents, incidents);
        assert_eq!(reconciled.generation, generation + 1);
    }

    #[test]
    fn reconcile_rolls_back_accepting_todo() {
        let workflow = spec_with(&["t1"], 3);
        let mut workflow_state = state(&workflow);
        let todo = workflow_state.todos.get_mut("t1").unwrap();
        todo.status = TodoStatus::Accepting;
        todo.candidate = Some(candidate_value());

        let reconciled = reconcile_interrupted(workflow_state);

        let todo = &reconciled.todos["t1"];
        assert_eq!(todo.status, TodoStatus::Interrupted);
        assert_eq!(todo.candidate, None);
        assert!(reconciled.active_todo_ids.is_empty());
    }

    #[test]
    fn reconcile_leaves_other_statuses_untouched() {
        let workflow = spec_with(&["passed", "revised", "pending", "active"], 3);
        let mut workflow_state = state(&workflow);
        for (id, status) in [
            ("passed", TodoStatus::Passed),
            ("revised", TodoStatus::NeedsRevision),
        ] {
            workflow_state.todos.get_mut(id).unwrap().status = status;
        }
        workflow_state.todos.get_mut("active").unwrap().status = TodoStatus::Running;
        workflow_state.active_todo_ids.insert("active".into());

        let reconciled = reconcile_interrupted(workflow_state);

        assert_eq!(reconciled.todos["passed"].status, TodoStatus::Passed);
        assert_eq!(
            reconciled.todos["revised"].status,
            TodoStatus::NeedsRevision
        );
        assert_eq!(reconciled.todos["pending"].status, TodoStatus::Pending);
        assert_eq!(reconciled.todos["active"].status, TodoStatus::Interrupted);
        assert!(reconciled.active_todo_ids.is_empty());
    }

    #[test]
    fn execution_failed_with_attempts_remaining_requests_revision() {
        let workflow = spec_with(&["t1"], 3);
        let workflow_state = dispatch(
            &workflow,
            state(&workflow),
            &[(
                DispatchTodo {
                    todo_id: "t1".into(),
                    context_mode: ContextMode::New,
                },
                "session".into(),
            )],
        )
        .unwrap();
        assert!(workflow_state.active_todo_ids.contains("t1"));

        let next = execution_failed(
            &workflow,
            workflow_state,
            "t1",
            "agent crashed".into(),
            false,
        )
        .unwrap();

        let todo = &next.todos["t1"];
        assert_eq!(todo.status, TodoStatus::NeedsRevision);
        assert_eq!(todo.last_error.as_deref(), Some("agent crashed"));
        assert!(!next.active_todo_ids.contains("t1"));
    }

    #[test]
    fn execution_failed_with_exhausted_attempts_marks_failed() {
        let workflow = spec_with(&["t1"], 1);
        let workflow_state = dispatch(
            &workflow,
            state(&workflow),
            &[(
                DispatchTodo {
                    todo_id: "t1".into(),
                    context_mode: ContextMode::New,
                },
                "session".into(),
            )],
        )
        .unwrap();

        let next = execution_failed(&workflow, workflow_state, "t1", "boom".into(), false).unwrap();

        assert_eq!(next.todos["t1"].status, TodoStatus::Failed);
        assert_eq!(next.todos["t1"].last_error.as_deref(), Some("boom"));
    }

    #[test]
    fn execution_failed_interrupted_marks_interrupted_even_when_exhausted() {
        let workflow = spec_with(&["t1"], 1);
        let workflow_state = dispatch(
            &workflow,
            state(&workflow),
            &[(
                DispatchTodo {
                    todo_id: "t1".into(),
                    context_mode: ContextMode::New,
                },
                "session".into(),
            )],
        )
        .unwrap();

        let next =
            execution_failed(&workflow, workflow_state, "t1", "ctrl-c".into(), true).unwrap();

        assert_eq!(next.todos["t1"].status, TodoStatus::Interrupted);
        assert_eq!(next.todos["t1"].last_error.as_deref(), Some("ctrl-c"));
        assert!(next.active_todo_ids.is_empty());
    }

    #[test]
    fn execution_failed_rejects_passed_todo() {
        let workflow = spec_with(&["t1"], 3);
        let mut workflow_state = state(&workflow);
        workflow_state.todos.get_mut("t1").unwrap().status = TodoStatus::Passed;

        let error = execution_failed(
            &workflow,
            workflow_state,
            "t1",
            "late failure".into(),
            false,
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("invalid TODO transition"));
    }

    fn spec_of(todos: Vec<TodoSpec>) -> WorkflowSpec {
        WorkflowSpec {
            schema_version: 1,
            id: "wf".into(),
            name: "wf".into(),
            objective: "objective".into(),
            constraints: Vec::new(),
            todos,
            metadata: serde_json::Value::Null,
        }
    }

    fn dep_todo(id: &str, deps: &[&str], max_attempts: u32) -> TodoSpec {
        let mut todo = spec_todo(id, max_attempts);
        todo.depends_on = deps.iter().map(|dep| (*dep).to_string()).collect();
        todo
    }

    fn dispatch_new(
        workflow: &WorkflowSpec,
        workflow_state: WorkflowState,
        id: &str,
    ) -> WorkflowState {
        dispatch(
            workflow,
            workflow_state,
            &[(
                DispatchTodo {
                    todo_id: id.into(),
                    context_mode: ContextMode::New,
                },
                "session".into(),
            )],
        )
        .unwrap()
    }

    fn at_accepting(workflow: &WorkflowSpec, id: &str) -> WorkflowState {
        let dispatched = dispatch_new(workflow, state(workflow), id);
        let readied = candidate(dispatched, id, candidate_value()).unwrap();
        accepting(readied, id).unwrap()
    }

    #[test]
    fn revise_from_accepting_marks_needs_revision_and_pins_context_mode() {
        let workflow = spec_with(&["t1"], 3);
        let workflow_state = at_accepting(&workflow, "t1");

        let next = revise(
            &workflow,
            workflow_state,
            "t1",
            "evidence did not match criteria".into(),
            ContextMode::Resume,
        )
        .unwrap();

        let todo = &next.todos["t1"];
        assert_eq!(todo.status, TodoStatus::NeedsRevision);
        assert_eq!(
            todo.last_error.as_deref(),
            Some("evidence did not match criteria")
        );
        assert_eq!(todo.next_context_mode, Some(ContextMode::Resume));
    }

    #[test]
    fn revise_rejects_pending_todo() {
        let workflow = spec_with(&["t1"], 3);
        let workflow_state = state(&workflow);

        let error = revise(
            &workflow,
            workflow_state,
            "t1",
            "premature".into(),
            ContextMode::New,
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("invalid TODO transition"));
    }

    #[test]
    fn revise_with_exhausted_attempts_marks_failed() {
        let workflow = spec_with(&["t1"], 1);
        let workflow_state = at_accepting(&workflow, "t1");

        let next = revise(
            &workflow,
            workflow_state,
            "t1",
            "still wrong".into(),
            ContextMode::Fork,
        )
        .unwrap();

        let todo = &next.todos["t1"];
        assert_eq!(todo.status, TodoStatus::Failed);
        assert_eq!(todo.last_error.as_deref(), Some("still wrong"));
        assert_eq!(todo.next_context_mode, Some(ContextMode::Fork));
    }

    #[test]
    fn rewind_invalidates_descendants_and_resets_milestone() {
        let workflow = spec_of(vec![
            dep_todo("m", &[], 3),
            dep_todo("c", &["m"], 3),
            dep_todo("g", &["c"], 3),
        ]);
        let mut workflow_state = state(&workflow);
        for id in ["m", "c", "g"] {
            let todo = workflow_state.todos.get_mut(id).unwrap();
            todo.status = TodoStatus::Passed;
            todo.accepted_generation = Some(7);
        }
        workflow_state.milestones.insert("m".into());
        workflow_state.active_todo_ids.insert("g".into());
        let epoch = workflow_state.world_epoch;

        let next = rewind(
            &workflow,
            workflow_state,
            "m",
            "ground truth drifted".into(),
        )
        .unwrap();

        let milestone = &next.todos["m"];
        assert_eq!(milestone.status, TodoStatus::Recovering);
        assert_eq!(milestone.accepted_generation, None);
        assert_eq!(
            milestone.last_error.as_deref(),
            Some("ground truth drifted")
        );
        for id in ["c", "g"] {
            assert_eq!(next.todos[id].status, TodoStatus::Invalidated, "{id}");
            assert_eq!(next.todos[id].accepted_generation, None, "{id}");
            assert_eq!(
                next.todos[id].last_error.as_deref(),
                Some("ground truth drifted")
            );
        }
        assert!(next.active_todo_ids.is_empty());
        assert_eq!(next.world_epoch, epoch + 1);
        assert_eq!(next.incidents.len(), 1);
        assert_eq!(next.incidents[0]["milestone_todo_id"], "m");
        assert_eq!(next.incidents[0]["reason"], "ground truth drifted");
        assert_eq!(
            next.incidents[0]["world_epoch"],
            serde_json::json!(epoch + 1)
        );
    }

    #[test]
    fn rewind_rejects_non_milestone() {
        let workflow = spec_with(&["t1", "t2"], 3);

        let error =
            rewind(&workflow, state(&workflow), "t1", "not a milestone".into()).unwrap_err();

        assert!(format!("{error:#}").contains("unknown milestone TODO t1"));
    }
}

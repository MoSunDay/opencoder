//! Pure state-machine guard tests for boundary fixes that need no store or
//! model: blocked-at-exhausted lands on Failed (and leaves runnable), a
//! Suspended terminal rolls back CandidateReady/Accepting, and dispatch keeps
//! the previous candidate so retry prompts can carry PREVIOUS_RECOVERY.

use opencoder_todos::{domain, transitions, types::*};

fn spec_with(max_attempts: u32) -> WorkflowSpec {
    WorkflowSpec {
        schema_version: 1,
        id: "wf-guards".into(),
        name: "guards".into(),
        objective: "objective".into(),
        constraints: Vec::new(),
        todos: vec![TodoSpec {
            id: "t1".into(),
            title: "one".into(),
            requirement_background: "required".into(),
            instructions: "do it".into(),
            depends_on: Vec::new(),
            agent: "act".into(),
            max_attempts,
            allowed_tools: vec![],
            acceptance: AcceptanceSpec {
                criteria: "candidate exists".into(),
                required_tool_calls: Vec::new(),
            },
            metadata: serde_json::Value::Null,
        }],
        metadata: serde_json::Value::Null,
    }
}

fn candidate_with(status: CandidateStatus) -> Candidate {
    Candidate {
        status,
        summary: "summary".into(),
        result: None,
        verification: "checked".into(),
        evidence_refs: Vec::new(),
        recovery_context: RecoveryContext {
            summary: "what I learned".into(),
            refs: Vec::new(),
        },
    }
}

fn dispatch_new(workflow: &WorkflowSpec, state: WorkflowState, id: &str) -> WorkflowState {
    transitions::dispatch(
        workflow,
        state,
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

/// M2: a blocked candidate report at exhausted attempts must land on Failed,
/// not NeedsRevision — otherwise runnable() keeps proposing a TODO that
/// validate_dispatch keeps refusing, suspending the workflow on every resume.
#[test]
fn blocked_candidate_at_exhausted_attempts_marks_failed() {
    let workflow = spec_with(1);
    let state = dispatch_new(
        &workflow,
        domain::initial_state(&workflow, "run".into(), "p".into()),
        "t1",
    );

    let next = transitions::candidate(
        &workflow,
        state,
        "t1",
        candidate_with(CandidateStatus::Blocked),
    )
    .unwrap();

    assert_eq!(next.todos["t1"].status, TodoStatus::Failed);
    // Failed todos never re-enter the runnable set, so the parent can decide
    // rewind/fail/complete instead of being trapped in a dispatch loop.
    assert!(domain::runnable(&workflow, &next).unwrap().is_empty());
}

/// M2 symmetry note: an interrupted self-report keeps Interrupted even at
/// exhausted attempts (same precedence as execution_failed) — the parent may
/// still rewind to reset the attempt budget.
#[test]
fn interrupted_candidate_stays_interrupted_at_exhausted_attempts() {
    let workflow = spec_with(1);
    let state = dispatch_new(
        &workflow,
        domain::initial_state(&workflow, "run".into(), "p".into()),
        "t1",
    );

    let next = transitions::candidate(
        &workflow,
        state,
        "t1",
        candidate_with(CandidateStatus::Interrupted),
    )
    .unwrap();

    assert_eq!(next.todos["t1"].status, TodoStatus::Interrupted);
}

/// M2 contrast: with attempts remaining, a blocked report still requests
/// revision.
#[test]
fn blocked_candidate_with_attempts_remaining_requests_revision() {
    let workflow = spec_with(2);
    let state = dispatch_new(
        &workflow,
        domain::initial_state(&workflow, "run".into(), "p".into()),
        "t1",
    );

    let next = transitions::candidate(
        &workflow,
        state,
        "t1",
        candidate_with(CandidateStatus::Blocked),
    )
    .unwrap();

    assert_eq!(next.todos["t1"].status, TodoStatus::NeedsRevision);
    assert_eq!(domain::runnable(&workflow, &next).unwrap(), vec!["t1"]);
}

/// L2: terminal(Suspended) must roll back CandidateReady and Accepting todos
/// (not only the Running ones in active_todo_ids), mirroring
/// reconcile_interrupted — a Suspended workflow must not leave items claiming
/// an in-progress status.
#[test]
fn suspended_terminal_rolls_back_candidate_ready_and_accepting() {
    for setup_status in [TodoStatus::CandidateReady, TodoStatus::Accepting] {
        let workflow = spec_with(3);
        let mut state = dispatch_new(
            &workflow,
            domain::initial_state(&workflow, "run".into(), "p".into()),
            "t1",
        );
        state.todos.get_mut("t1").unwrap().status = setup_status;
        state.todos.get_mut("t1").unwrap().candidate =
            Some(candidate_with(CandidateStatus::Candidate));

        let next = transitions::terminal(state, WorkflowStatus::Suspended, "stop".into()).unwrap();

        let todo = &next.todos["t1"];
        assert_eq!(todo.status, TodoStatus::Interrupted, "{setup_status:?}");
        assert_eq!(todo.last_error.as_deref(), Some("stop"));
        assert_eq!(todo.candidate, None, "stale candidate must be cleared");
        assert!(next.active_todo_ids.is_empty());
        assert_eq!(next.status, WorkflowStatus::Suspended);
    }
}

/// M3: dispatch must KEEP the previous attempt's candidate — the execution
/// snapshot is taken after dispatch and focused_prompt reads it to surface
/// PREVIOUS_RECOVERY on a retry. last_error / next_context_mode are still
/// cleared for the fresh attempt.
#[test]
fn dispatch_keeps_previous_candidate_for_retry_recovery_context() {
    let workflow = spec_with(3);
    let state = domain::initial_state(&workflow, "run".into(), "p".into());
    let state = dispatch_new(&workflow, state, "t1");
    let state = transitions::candidate(
        &workflow,
        state,
        "t1",
        candidate_with(CandidateStatus::Blocked),
    )
    .unwrap();
    assert_eq!(state.todos["t1"].status, TodoStatus::NeedsRevision);
    assert!(state.todos["t1"].candidate.is_some());

    let redispatched = transitions::dispatch(
        &workflow,
        state.clone(),
        &[(
            DispatchTodo {
                todo_id: "t1".into(),
                context_mode: ContextMode::Resume,
            },
            state.todos["t1"].active_session_id.clone().unwrap(),
        )],
    )
    .unwrap();

    let todo = &redispatched.todos["t1"];
    assert_eq!(todo.status, TodoStatus::Running);
    assert_eq!(todo.attempt, 2);
    assert!(
        todo.candidate.is_some(),
        "retry prompt needs the previous candidate's recovery_context"
    );
    assert_eq!(todo.last_error, None);
    assert_eq!(todo.next_context_mode, None);
}

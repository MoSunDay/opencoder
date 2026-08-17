//! Bug #16b: `transitions::validate_dispatch` is the pure, side-effect-free
//! half of `dispatch`. Every rejection `dispatch` enforces must be surfaced
//! by the dry-run first — that is what lets `drive_inner` correct a bad
//! parent decision before any state changes.

use opencoder_todos::{
    transitions,
    types::{ContextMode, DispatchTodo, TodoSpec, WorkflowSpec},
};

fn todo_spec(id: &str, deps: &[&str], max_attempts: u32) -> TodoSpec {
    TodoSpec {
        id: id.into(),
        title: format!("title {id}"),
        requirement_background: "background".into(),
        instructions: "instructions".into(),
        depends_on: deps.iter().map(|dep| (*dep).to_string()).collect(),
        agent: "act".into(),
        max_attempts,
        acceptance: opencoder_todos::types::AcceptanceSpec {
            criteria: "done".into(),
            required_tool_calls: Vec::new(),
        },
        metadata: serde_json::Value::Null,
    }
}

fn spec(todos: Vec<TodoSpec>) -> WorkflowSpec {
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

fn fresh_state(workflow: &WorkflowSpec) -> opencoder_todos::WorkflowState {
    opencoder_todos::domain::initial_state(workflow, "wf-run".into(), "parent".into())
}

fn validate(workflow: &WorkflowSpec, request: DispatchTodo) -> anyhow::Result<()> {
    let state = fresh_state(workflow);
    let assignments = vec![(request.clone(), "s1".into())];
    transitions::validate_dispatch(workflow, &state, &[request], &assignments)
}

#[test]
fn validate_dispatch_accepts_a_runnable_first_attempt() {
    let workflow = spec(vec![todo_spec("t1", &[], 3)]);

    validate(
        &workflow,
        DispatchTodo {
            todo_id: "t1".into(),
            context_mode: ContextMode::New,
        },
    )
    .unwrap();
}

#[test]
fn validate_dispatch_rejects_exhausted_todo_and_keeps_dispatch_rejecting_it() {
    let workflow = spec(vec![todo_spec("t1", &[], 1)]);
    let mut state = fresh_state(&workflow);
    state.todos.get_mut("t1").unwrap().attempt = 1;
    let request = DispatchTodo {
        todo_id: "t1".into(),
        context_mode: ContextMode::Fork,
    };
    let assignments = vec![(request.clone(), "s1".into())];

    let error =
        transitions::validate_dispatch(&workflow, &state, &[request], &assignments).unwrap_err();

    assert!(format!("{error}").contains("exhausted max_attempts"));
    assert_eq!(
        state.todos["t1"].attempt, 1,
        "dry-run validation must not mutate state"
    );
    assert!(
        transitions::dispatch(&workflow, state, &assignments).is_err(),
        "dispatch keeps rejecting exactly what validate_dispatch rejects"
    );
}

#[test]
fn validate_dispatch_rejects_resume_without_session() {
    let workflow = spec(vec![todo_spec("t1", &[], 3)]);

    let error = validate(
        &workflow,
        DispatchTodo {
            todo_id: "t1".into(),
            context_mode: ContextMode::Resume,
        },
    )
    .unwrap_err();

    assert!(format!("{error}").contains("no session to resume"));
}

#[test]
fn validate_dispatch_rejects_todo_with_unpassed_dependency() {
    let workflow = spec(vec![todo_spec("a", &[], 3), todo_spec("b", &["a"], 3)]);

    let error = validate(
        &workflow,
        DispatchTodo {
            todo_id: "b".into(),
            context_mode: ContextMode::New,
        },
    )
    .unwrap_err();

    assert!(format!("{error}").contains("TODO b is not runnable"));
}

#[test]
fn validate_dispatch_rejects_context_mode_that_breaks_revision_contract() {
    let workflow = spec(vec![todo_spec("t1", &[], 3)]);
    let mut state = fresh_state(&workflow);
    state.todos.get_mut("t1").unwrap().next_context_mode = Some(ContextMode::Fork);
    let request = DispatchTodo {
        todo_id: "t1".into(),
        context_mode: ContextMode::New,
    };
    let assignments = vec![(request.clone(), "s1".into())];

    let error =
        transitions::validate_dispatch(&workflow, &state, &[request], &assignments).unwrap_err();

    assert!(format!("{error}").contains("must use context_mode Fork"));
}

#[test]
fn validate_dispatch_rejects_duplicate_dispatch_in_one_request() {
    let workflow = spec(vec![todo_spec("t1", &[], 3)]);
    let state = fresh_state(&workflow);
    let request = DispatchTodo {
        todo_id: "t1".into(),
        context_mode: ContextMode::New,
    };
    let assignments = vec![
        (request.clone(), "s1".into()),
        (request.clone(), "s2".into()),
    ];

    let error = transitions::validate_dispatch(
        &workflow,
        &state,
        &[request.clone(), request],
        &assignments,
    )
    .unwrap_err();

    assert!(format!("{error}").contains("dispatched more than once"));
}

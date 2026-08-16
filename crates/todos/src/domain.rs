use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::{bail, Context, Result};

use crate::types::*;

pub fn validate_spec(spec: &WorkflowSpec) -> Result<()> {
    if spec.schema_version != 1 {
        bail!("unsupported todos schema_version {}", spec.schema_version);
    }
    for (name, value) in [
        ("id", spec.id.as_str()),
        ("name", spec.name.as_str()),
        ("objective", spec.objective.as_str()),
    ] {
        if value.trim().is_empty() {
            bail!("workflow {name} cannot be empty");
        }
    }
    if spec.todos.is_empty() {
        bail!("workflow must contain at least one TODO");
    }
    let ids: HashSet<&str> = spec.todos.iter().map(|todo| todo.id.as_str()).collect();
    if ids.len() != spec.todos.len() {
        bail!("TODO ids must be unique");
    }
    for todo in &spec.todos {
        if todo.id.trim().is_empty()
            || todo.title.trim().is_empty()
            || todo.requirement_background.trim().is_empty()
            || todo.instructions.trim().is_empty()
            || todo.acceptance.criteria.trim().is_empty()
        {
            bail!("TODO {} has an empty required field", todo.id);
        }
        if todo.max_attempts == 0 {
            bail!("TODO {} max_attempts must be positive", todo.id);
        }
        for dep in &todo.depends_on {
            if dep == &todo.id || !ids.contains(dep.as_str()) {
                bail!("TODO {} has invalid dependency {dep}", todo.id);
            }
        }
        for call in &todo.acceptance.required_tool_calls {
            if call.name.trim().is_empty() || !call.arguments_contains.is_object() {
                bail!("TODO {} has an invalid required tool call", todo.id);
            }
        }
    }
    reject_cycles(spec)
}

fn reject_cycles(spec: &WorkflowSpec) -> Result<()> {
    let deps: HashMap<&str, Vec<&str>> = spec
        .todos
        .iter()
        .map(|todo| {
            (
                todo.id.as_str(),
                todo.depends_on.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    fn visit<'a>(
        id: &'a str,
        deps: &HashMap<&'a str, Vec<&'a str>>,
        visiting: &mut HashSet<&'a str>,
        done: &mut HashSet<&'a str>,
    ) -> Result<()> {
        if done.contains(id) {
            return Ok(());
        }
        if !visiting.insert(id) {
            bail!("TODO dependency graph contains a cycle at {id}");
        }
        for dep in &deps[id] {
            visit(dep, deps, visiting, done)?;
        }
        visiting.remove(id);
        done.insert(id);
        Ok(())
    }
    let mut visiting = HashSet::new();
    let mut done = HashSet::new();
    for id in deps.keys() {
        visit(id, &deps, &mut visiting, &mut done)?;
    }
    Ok(())
}

pub fn initial_state(
    spec: &WorkflowSpec,
    workflow_id: String,
    parent_session_id: String,
) -> WorkflowState {
    WorkflowState {
        workflow_id,
        parent_session_id,
        status: WorkflowStatus::Pending,
        generation: 0,
        world_epoch: 0,
        active_todo_ids: BTreeSet::new(),
        todos: spec
            .todos
            .iter()
            .map(|todo| {
                (
                    todo.id.clone(),
                    TodoState {
                        status: TodoStatus::Pending,
                        attempt: 0,
                        active_session_id: None,
                        session_history: Vec::new(),
                        candidate: None,
                        last_error: None,
                        accepted_generation: None,
                        next_context_mode: None,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>(),
        milestones: BTreeSet::new(),
        incidents: Vec::new(),
        terminal_reason: None,
    }
}

pub fn runnable(spec: &WorkflowSpec, state: &WorkflowState) -> Result<Vec<String>> {
    let mut runnable = Vec::new();
    for todo in &spec.todos {
        let current = state_todo(state, &todo.id)?;
        let eligible = matches!(
            current.status,
            TodoStatus::Pending
                | TodoStatus::NeedsRevision
                | TodoStatus::Interrupted
                | TodoStatus::Invalidated
                | TodoStatus::Recovering
        ) && deps_passed(state, todo)?;
        if eligible {
            runnable.push(todo.id.clone());
        }
    }
    Ok(runnable)
}

fn deps_passed(state: &WorkflowState, todo: &TodoSpec) -> Result<bool> {
    for dep in &todo.depends_on {
        if state_todo(state, dep)?.status != TodoStatus::Passed {
            return Ok(false);
        }
    }
    Ok(true)
}

fn state_todo<'a>(state: &'a WorkflowState, id: &str) -> Result<&'a TodoState> {
    state
        .todos
        .get(id)
        .with_context(|| format!("state missing TODO {id}"))
}

pub fn descendants(spec: &WorkflowSpec, root: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    loop {
        let before = out.len();
        for todo in &spec.todos {
            if todo
                .depends_on
                .iter()
                .any(|dep| dep == root || out.contains(dep))
            {
                out.insert(todo.id.clone());
            }
        }
        if out.len() == before {
            break;
        }
    }
    out
}

pub fn json_contains(actual: &serde_json::Value, expected: &serde_json::Value) -> bool {
    match (actual, expected) {
        (serde_json::Value::Object(a), serde_json::Value::Object(e)) => e
            .iter()
            .all(|(key, value)| a.get(key).is_some_and(|got| json_contains(got, value))),
        (serde_json::Value::Array(a), serde_json::Value::Array(e)) => e
            .iter()
            .all(|expected| a.iter().any(|actual| json_contains(actual, expected))),
        _ => actual == expected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_json_subset_is_supported() {
        assert!(json_contains(
            &serde_json::json!({"a":{"b":1,"c":2}}),
            &serde_json::json!({"a":{"b":1}})
        ));
    }

    fn valid_todo(id: &str, deps: &[&str]) -> TodoSpec {
        TodoSpec {
            id: id.into(),
            title: format!("title {id}"),
            requirement_background: "background".into(),
            instructions: "instructions".into(),
            depends_on: deps.iter().map(|dep| (*dep).to_string()).collect(),
            agent: "act".into(),
            max_attempts: 2,
            acceptance: AcceptanceSpec {
                criteria: "done".into(),
                required_tool_calls: Vec::new(),
            },
            metadata: serde_json::Value::Null,
        }
    }

    fn valid_spec(todos: Vec<TodoSpec>) -> WorkflowSpec {
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

    #[test]
    fn validate_spec_rejects_unknown_dependency() {
        let workflow = valid_spec(vec![valid_todo("a", &[]), valid_todo("b", &["ghost"])]);

        let error = validate_spec(&workflow).unwrap_err();

        assert!(format!("{error}").contains("invalid dependency ghost"));
    }

    #[test]
    fn validate_spec_rejects_self_dependency() {
        let workflow = valid_spec(vec![valid_todo("a", &["a"])]);

        let error = validate_spec(&workflow).unwrap_err();

        assert!(format!("{error}").contains("invalid dependency a"));
    }

    #[test]
    fn validate_spec_rejects_zero_max_attempts() {
        let mut todo = valid_todo("a", &[]);
        todo.max_attempts = 0;
        let workflow = valid_spec(vec![todo]);

        let error = validate_spec(&workflow).unwrap_err();

        assert!(format!("{error}").contains("max_attempts must be positive"));
    }

    #[test]
    fn validate_spec_rejects_empty_instructions() {
        let mut todo = valid_todo("a", &[]);
        todo.instructions = "   ".into();
        let workflow = valid_spec(vec![todo]);

        let error = validate_spec(&workflow).unwrap_err();

        assert!(format!("{error}").contains("empty required field"));
    }

    #[test]
    fn validate_spec_rejects_non_object_tool_arguments() {
        let mut todo = valid_todo("a", &[]);
        todo.acceptance.required_tool_calls = vec![RequiredToolCall {
            name: "mcp__fk__tap".into(),
            arguments_contains: serde_json::json!(["not-an-object"]),
            result_ok: true,
        }];
        let workflow = valid_spec(vec![todo]);

        let error = validate_spec(&workflow).unwrap_err();

        assert!(format!("{error}").contains("invalid required tool call"));
    }

    #[test]
    fn validate_spec_accepts_diamond_and_runnable_orders_correctly() {
        let workflow = valid_spec(vec![
            valid_todo("a", &[]),
            valid_todo("b", &["a"]),
            valid_todo("c", &["a"]),
            valid_todo("d", &["b", "c"]),
        ]);

        validate_spec(&workflow).unwrap();
        let mut workflow_state = initial_state(&workflow, "run".into(), "parent".into());
        assert_eq!(runnable(&workflow, &workflow_state).unwrap(), vec!["a"]);

        workflow_state.todos.get_mut("a").unwrap().status = TodoStatus::Passed;
        assert_eq!(
            runnable(&workflow, &workflow_state).unwrap(),
            vec!["b", "c"]
        );

        for id in ["b", "c"] {
            workflow_state.todos.get_mut(id).unwrap().status = TodoStatus::Passed;
        }
        assert_eq!(runnable(&workflow, &workflow_state).unwrap(), vec!["d"]);

        workflow_state.todos.get_mut("d").unwrap().status = TodoStatus::Passed;
        assert!(runnable(&workflow, &workflow_state).unwrap().is_empty());
    }
}

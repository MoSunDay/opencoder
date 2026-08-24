use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::{bail, Context, Result};
use opencoder_core::resolve_agent;

use crate::types::*;

pub fn validate_spec(spec: &WorkflowSpec) -> Result<()> {
    if !matches!(spec.schema_version, 1 | 2) {
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
        if spec.schema_version >= 2 && todo.allowed_tools.is_empty() {
            bail!("TODO {} must declare allowed_tools in schema v2", todo.id);
        }
        if todo.allowed_tools.iter().any(|name| name.trim().is_empty()) {
            bail!("TODO {} has an empty allowed tool", todo.id);
        }
        // Path safety: todo ids feed file paths in the --debug projection
        // (`sessions/todos/<id>/attempt-NNN.json`, `task-info/todos/<id>.json`),
        // so traversal-shaped ids are rejected at spec-submission time and
        // debug_dump can keep trusting the validated spec.
        if todo.id.contains(['/', '\\', '\0']) || todo.id.contains("..") {
            bail!("TODO {} id is not path-safe", todo.id);
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
        // Bug #16d: the same agent checks execution.rs applies at runtime,
        // enforced at spec-submission time so a typo'd agent name fails fast
        // instead of suspending the workflow on the first dispatch.
        let agent = resolve_agent(&todo.agent)
            .with_context(|| format!("TODO {} has unknown agent {}", todo.id, todo.agent))?;
        if !agent.is_primary() {
            bail!(
                "TODO {} agent {} must be a primary agent",
                todo.id,
                todo.agent
            );
        }
        if agent.name == "workflow" {
            bail!("TODO {} cannot use the workflow agent", todo.id);
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
    // Iterative tri-color DFS with an explicit stack: the spec is
    // user-supplied JSON and a long dependency chain must not be able to
    // abort the process by overflowing the recursion stack.
    let mut visiting: HashSet<&str> = HashSet::new();
    let mut done: HashSet<&str> = HashSet::new();
    for root in deps.keys() {
        if done.contains(root) {
            continue;
        }
        visiting.insert(root);
        let mut stack: Vec<(&str, usize)> = vec![(root, 0)];
        while let Some(&(id, index)) = stack.last() {
            let children = &deps[id];
            if index < children.len() {
                let child = children[index];
                stack.last_mut().expect("non-empty").1 += 1;
                if done.contains(child) {
                    continue;
                }
                if !visiting.insert(child) {
                    bail!("TODO dependency graph contains a cycle at {child}");
                }
                stack.push((child, 0));
            } else {
                stack.pop();
                visiting.remove(id);
                done.insert(id);
            }
        }
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
            allowed_tools: vec![],
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
    fn validate_spec_rejects_unknown_agent() {
        let mut todo = valid_todo("a", &[]);
        todo.agent = "ghost-agent".into();
        let workflow = valid_spec(vec![todo]);

        let error = validate_spec(&workflow).unwrap_err();

        assert!(format!("{error}").contains("TODO a has unknown agent ghost-agent"));
    }

    #[test]
    fn validate_spec_rejects_workflow_agent() {
        let mut todo = valid_todo("a", &[]);
        todo.agent = "workflow".into();
        let workflow = valid_spec(vec![todo]);

        let error = validate_spec(&workflow).unwrap_err();

        assert!(format!("{error}").contains("TODO a cannot use the workflow agent"));
    }

    #[test]
    fn validate_spec_rejects_non_primary_agents() {
        for agent in ["explore", "build"] {
            let mut todo = valid_todo("a", &[]);
            todo.agent = agent.into();
            let workflow = valid_spec(vec![todo]);

            let error = validate_spec(&workflow).unwrap_err();

            assert!(
                format!("{error}")
                    .contains(&format!("TODO a agent {agent} must be a primary agent")),
                "subagent {agent} must be rejected at submission time"
            );
        }
    }

    #[test]
    fn validate_spec_accepts_act_agent() {
        let mut todo = valid_todo("a", &[]);
        todo.agent = "act".into();
        let workflow = valid_spec(vec![todo]);

        validate_spec(&workflow).unwrap();
    }

    #[test]
    fn validate_spec_rejects_path_unsafe_todo_ids() {
        for bad in ["a/b", "a\\b", "a..b", "..", "a\u{0}b"] {
            let workflow = valid_spec(vec![valid_todo(bad, &[])]);
            let error = validate_spec(&workflow)
                .err()
                .unwrap_or_else(|| panic!("id {bad:?} must be rejected"));
            assert!(
                format!("{error}").contains("not path-safe"),
                "id {bad:?} rejected for the wrong reason: {error}"
            );
        }
    }

    #[test]
    fn validate_spec_survives_deep_dependency_chain() {
        // 30_000-long chain: a recursive DFS visitor would abort the process
        // by overflowing the stack; the iterative one must accept the
        // acyclic spec (and reject a cycle planted at the deep end).
        let depth = 30_000;
        let mut todos = Vec::with_capacity(depth);
        for i in 0..depth {
            let mut todo = valid_todo(&format!("t{i}"), &[]);
            if i > 0 {
                todo.depends_on = vec![format!("t{}", i - 1)];
            }
            todos.push(todo);
        }
        validate_spec(&valid_spec(todos)).expect("deep acyclic chain is valid");

        let mut cyclic = Vec::with_capacity(depth);
        for i in 0..depth {
            let mut todo = valid_todo(&format!("t{i}"), &[]);
            if i > 0 {
                todo.depends_on = vec![format!("t{}", i - 1)];
            } else {
                todo.depends_on = vec![format!("t{}", depth - 1)];
            }
            cyclic.push(todo);
        }
        let error = validate_spec(&valid_spec(cyclic)).unwrap_err();
        assert!(format!("{error}").contains("cycle"));
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

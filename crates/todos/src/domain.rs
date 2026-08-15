use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::{bail, Result};

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

pub fn runnable(spec: &WorkflowSpec, state: &WorkflowState) -> Vec<String> {
    spec.todos
        .iter()
        .filter(|todo| {
            let current = &state.todos[&todo.id];
            matches!(
                current.status,
                TodoStatus::Pending
                    | TodoStatus::NeedsRevision
                    | TodoStatus::Interrupted
                    | TodoStatus::Invalidated
                    | TodoStatus::Recovering
            ) && todo
                .depends_on
                .iter()
                .all(|dep| state.todos[dep].status == TodoStatus::Passed)
        })
        .map(|todo| todo.id.clone())
        .collect()
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
}

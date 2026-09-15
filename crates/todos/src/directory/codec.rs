use crate::types::{AcceptanceSpec, RequiredToolCall, TodoSpec, WorkflowSpec};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub type Files = BTreeMap<String, String>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub path: String,
    pub message: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowFile {
    schema_version: u32,
    id: String,
    name: String,
    #[serde(default)]
    constraints: Vec<String>,
    todos: Vec<String>,
    #[serde(default)]
    metadata: Value,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskFile {
    title: String,
    agent: String,
    max_attempts: u32,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    required_tool_calls: Vec<RequiredToolCall>,
    #[serde(default)]
    metadata: Value,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    env: Option<String>,
}

fn issue(path: &str, message: impl ToString) -> Diagnostic {
    Diagnostic {
        path: path.into(),
        message: message.to_string(),
        line: 1,
        column: 1,
    }
}

fn parse<T: serde::de::DeserializeOwned>(
    files: &Files,
    path: &str,
    errors: &mut Vec<Diagnostic>,
) -> Option<T> {
    match files.get(path) {
        None => {
            errors.push(issue(path, "缺少必需文件"));
            None
        }
        Some(text) => match serde_json::from_str(text) {
            Ok(value) => Some(value),
            Err(error) => {
                errors.push(Diagnostic {
                    path: path.into(),
                    message: error.to_string(),
                    line: error.line(),
                    column: error.column(),
                });
                None
            }
        },
    }
}

fn markdown(files: &Files, path: &str, errors: &mut Vec<Diagnostic>) -> String {
    match files.get(path) {
        Some(value) if !value.trim().is_empty() => value.clone(),
        _ => {
            errors.push(issue(path, "必需的 Markdown 内容不能为空"));
            String::new()
        }
    }
}

pub fn decode(files: &Files) -> Result<(WorkflowSpec, Option<String>), Vec<Diagnostic>> {
    let mut errors = Vec::new();
    let workflow: Option<WorkflowFile> = parse(files, "workflow.json", &mut errors);
    let binding: Option<Binding> = parse(files, "env.json", &mut errors);
    let objective = markdown(files, "objective.md", &mut errors);
    let mut expected: BTreeSet<String> = ["workflow.json", "env.json", "objective.md"]
        .map(str::to_owned)
        .into();
    let mut todos = Vec::new();
    if let Some(workflow) = &workflow {
        let mut ids = BTreeSet::new();
        for id in &workflow.todos {
            if opencoder_core::share_fs::validate_share_name(id).is_err() || id.contains("..") {
                errors.push(issue("workflow.json", format!("非法 TODO 目录名: {id}")));
                continue;
            }
            if !ids.insert(id) {
                errors.push(issue("workflow.json", format!("重复 TODO: {id}")));
                continue;
            }
            let base = format!("todos/{id}");
            for file in [
                "task.json",
                "context.md",
                "instructions.md",
                "acceptance.md",
            ] {
                expected.insert(format!("{base}/{file}"));
            }
            let task: Option<TaskFile> = parse(files, &format!("{base}/task.json"), &mut errors);
            let background = markdown(files, &format!("{base}/context.md"), &mut errors);
            let instructions = markdown(files, &format!("{base}/instructions.md"), &mut errors);
            let criteria = markdown(files, &format!("{base}/acceptance.md"), &mut errors);
            if let Some(task) = task {
                todos.push(TodoSpec {
                    id: id.clone(),
                    title: task.title,
                    requirement_background: background,
                    instructions,
                    depends_on: task.depends_on,
                    agent: task.agent,
                    max_attempts: task.max_attempts,
                    acceptance: AcceptanceSpec {
                        criteria,
                        required_tool_calls: task.required_tool_calls,
                    },
                    metadata: task.metadata,
                });
            }
        }
        for path in files.keys().filter(|path| !expected.contains(*path)) {
            errors.push(issue(
                path,
                "文件不属于 TODO 框架定义，或所属 TODO 未列入 workflow.json",
            ));
        }
    }
    if let Some(env) = binding.as_ref().and_then(|b| b.env.as_deref()) {
        if let Err(error) = opencoder_core::share_fs::validate_share_name(env) {
            errors.push(issue("env.json", error));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let workflow = workflow.expect("validated workflow");
    let spec = WorkflowSpec {
        schema_version: workflow.schema_version,
        id: workflow.id,
        name: workflow.name,
        objective,
        constraints: workflow.constraints,
        todos,
        metadata: workflow.metadata,
    };
    if let Err(error) = crate::domain::validate_spec(&spec) {
        let message = format!("{error:#}");
        let path = spec
            .todos
            .iter()
            .find(|todo| message.starts_with(&format!("TODO {} ", todo.id)))
            .map(|todo| format!("todos/{}/task.json", todo.id))
            .unwrap_or_else(|| "workflow.json".into());
        return Err(vec![issue(&path, message)]);
    }
    Ok((spec, binding.and_then(|b| b.env)))
}

pub fn validate(files: &Files) -> Vec<Diagnostic> {
    decode(files).err().unwrap_or_default()
}

pub fn encode(spec: &WorkflowSpec, env: Option<&str>) -> anyhow::Result<Files> {
    crate::domain::validate_spec(spec)?;
    let mut files = Files::new();
    files.insert(
        "workflow.json".into(),
        serde_json::to_string_pretty(&WorkflowFile {
            schema_version: spec.schema_version,
            id: spec.id.clone(),
            name: spec.name.clone(),
            constraints: spec.constraints.clone(),
            todos: spec.todos.iter().map(|t| t.id.clone()).collect(),
            metadata: spec.metadata.clone(),
        })?,
    );
    files.insert("objective.md".into(), spec.objective.clone());
    files.insert(
        "env.json".into(),
        serde_json::to_string_pretty(&json!({"env":env}))?,
    );
    for todo in &spec.todos {
        let base = format!("todos/{}", todo.id);
        files.insert(
            format!("{base}/task.json"),
            serde_json::to_string_pretty(&TaskFile {
                title: todo.title.clone(),
                agent: todo.agent.clone(),
                max_attempts: todo.max_attempts,
                depends_on: todo.depends_on.clone(),
                required_tool_calls: todo.acceptance.required_tool_calls.clone(),
                metadata: todo.metadata.clone(),
            })?,
        );
        files.insert(
            format!("{base}/context.md"),
            todo.requirement_background.clone(),
        );
        files.insert(format!("{base}/instructions.md"), todo.instructions.clone());
        files.insert(
            format!("{base}/acceptance.md"),
            todo.acceptance.criteria.clone(),
        );
    }
    Ok(files)
}

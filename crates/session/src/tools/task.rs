use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{json, Tool, ToolContext, ToolOutput};
use serde_json::Value;

pub struct TaskTool;

#[async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &str {
        "task"
    }
    fn description(&self) -> &str {
        // Canonical act-mode description. Schema generation routes through
        // [`description_for`] / [`parameters_for`] which adapt to the owning
        // agent's kind (sandbox mode drops `build`); this trait method is a
        // fallback for any direct `tool.description()` consumer.
        "Launch a subagent to handle a delegated task in isolation. \
         The subagent has its own message history and tools, and returns a final summary. \
         Use subagent_type \"explore\" for read-only codebase investigation (search/read), \
         or \"build\" for implementation work (bash/edit)."
    }
    fn parameters(&self) -> Value {
        parameters_for(false)
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        Ok(ToolOutput::err(
            "task tool is handled by the runner and should not be called directly",
        ))
    }
}

/// Description of the `task` tool. `explore` is always advertised; `build`
/// is shown only when `hide_build` is false (sandbox mode always hides it,
/// as does any agent while the task-plan skill is active).
pub fn description_for(hide_build: bool) -> String {
    let prefix = "Launch a subagent to handle a delegated task in isolation. \
                  The subagent has its own message history and tools, and returns a final summary. \
                  Use subagent_type \"explore\" for read-only codebase investigation \
                  (search/read)";
    let build_clause = if hide_build {
        String::new()
    } else {
        ", or \"build\" for implementation work (bash/edit).".to_string()
    };
    format!("{prefix}{build_clause}")
}

/// Parameter schema of the `task` tool, parameterised identically to
/// [`description_for`]. The `subagent_type` description only lists the kinds
/// the model may actually use.
pub fn parameters_for(hide_build: bool) -> Value {
    let mut subagent_type_desc = String::from("Agent type: \"explore\" (read-only)");
    if !hide_build {
        subagent_type_desc.push_str(", or \"build\" (full tools)");
    }
    subagent_type_desc.push_str(". Defaults to \"explore\".");

    let mut props = serde_json::Map::new();
    props.insert(
        "description".into(),
        json::prop_str("Short (3-5 word) description of the task."),
    );
    props.insert(
        "prompt".into(),
        json::prop_str("Full instructions for the subagent."),
    );
    props.insert("subagent_type".into(), json::prop_str(&subagent_type_desc));
    json::object_schema(Value::Object(props), &["description", "prompt"])
}

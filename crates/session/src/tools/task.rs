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
        "Launch a subagent to handle a delegated task in isolation. The subagent has its own message history and tools, and returns a final summary. Use subagent_type \"explore\" for read-only codebase investigation (read/glob/grep/ls/bash), or \"build\" for implementation work (read/write/edit/bash/glob/grep/ls)."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert(
            "description".into(),
            json::prop_str("Short (3-5 word) description of the task."),
        );
        props.insert(
            "prompt".into(),
            json::prop_str("Full instructions for the subagent."),
        );
        props.insert("subagent_type".into(), json::prop_str("Agent type: \"explore\" (read-only) or \"build\" (full tools). Defaults to \"explore\"."));
        json::object_schema(Value::Object(props), &["description", "prompt"])
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        Ok(ToolOutput::err(
            "task tool is handled by the runner and should not be called directly",
        ))
    }
}

/// Description of the `task` tool as exposed to the model while in **plan mode**.
///
/// Plan mode must never reveal the `build` subagent type to the LLM: it advertises
/// only the read-only `explore` subagent. This mirrors [`TaskTool::description`]
/// minus the `build` clause.
pub fn description_plan() -> &'static str {
    "Launch a subagent to handle a delegated task in isolation. The subagent has its own message history and tools, and returns a final summary. Use subagent_type \"explore\" for read-only codebase investigation (read/glob/grep/ls/bash)."
}

/// Parameter schema of the `task` tool as exposed to the model while in **plan mode**.
///
/// Identical to [`TaskTool::parameters`] except the `subagent_type` description only
/// mentions `explore`, so the model is never told that `build` exists.
pub fn parameters_plan() -> Value {
    let mut props = serde_json::Map::new();
    props.insert(
        "description".into(),
        json::prop_str("Short (3-5 word) description of the task."),
    );
    props.insert(
        "prompt".into(),
        json::prop_str("Full instructions for the subagent."),
    );
    props.insert("subagent_type".into(), json::prop_str("Agent type: \"explore\" (read-only). Defaults to \"explore\"."));
    json::object_schema(Value::Object(props), &["description", "prompt"])
}

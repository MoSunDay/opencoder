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
        // Canonical full (act + tools-on) description. Schema generation routes
        // through [`description_for`] / [`parameters_for`] which adapt to the
        // owning agent's kind and the `tools_subagent` capability; this trait
        // method is a fallback for any direct `tool.description()` consumer.
        "Launch a subagent to handle a delegated task in isolation. The subagent has its own message history and tools, and returns a final summary. Use subagent_type \"explore\" for read-only codebase investigation (read/glob/grep/ls/bash), \"build\" for implementation work (read/write/edit/bash/glob/grep/ls), or \"tools\" for browser (web_fetch/web_search) and computer-use capabilities plus read-only filesystem tools."
    }
    fn parameters(&self) -> Value {
        parameters_for(false, true)
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        Ok(ToolOutput::err(
            "task tool is handled by the runner and should not be called directly",
        ))
    }
}

/// Description of the `task` tool, parameterised by agent kind and the
/// `tools_subagent` capability. Four combinations:
/// - act  + tools-on : `explore`, `build`, `tools`
/// - act  + tools-off: `explore`, `build`
/// - plan + tools-on : `explore`, `tools`
/// - plan + tools-off: `explore`
///
/// `plan` mode never reveals `build`; a disabled capability never reveals
/// `tools`. This keeps the read-only / capability contracts at the *schema*
/// layer, before any runtime guard in `run_subagent` ever fires.
pub fn description_for(plan: bool, tools_on: bool) -> String {
    let prefix = "Launch a subagent to handle a delegated task in isolation. \
                  The subagent has its own message history and tools, and returns a final summary. \
                  Use subagent_type \"explore\" for read-only codebase investigation \
                  (read/glob/grep/ls/bash)";
    let build_clause = if plan {
        String::new()
    } else {
        ", \"build\" for implementation work (read/write/edit/bash/glob/grep/ls)".to_string()
    };
    let suffix = if tools_on {
        ", or \"tools\" for browser (web_fetch/web_search) and computer-use capabilities \
                     plus read-only filesystem tools."
    } else {
        "."
    };
    format!("{prefix}{build_clause}{suffix}")
}

/// Parameter schema of the `task` tool, parameterised identically to
/// [`description_for`]. The `subagent_type` description only lists the kinds
/// the model may actually use.
pub fn parameters_for(plan: bool, tools_on: bool) -> Value {
    let mut subagent_type_desc = String::from("Agent type: \"explore\" (read-only)");
    if !plan {
        subagent_type_desc.push_str(", \"build\" (full tools)");
    }
    if tools_on {
        subagent_type_desc.push_str(", or \"tools\" (browser + computer-use)");
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

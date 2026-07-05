use anyhow::Result;
use async_trait::async_trait;
use opencode_core::{json, Tool, ToolContext, ToolOutput};
use serde_json::Value;

pub struct TaskTool;

#[async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &str { "task" }
    fn description(&self) -> &str {
        "Launch a subagent to handle a delegated task in isolation. The subagent has its own message history and read/write/bash tools, and returns a final summary. Use for exploration or focused sub-tasks. Specify subagent_type (e.g. \"subagent\")."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert("description".into(), json::prop_str("Short (3-5 word) description of the task."));
        props.insert("prompt".into(), json::prop_str("Full instructions for the subagent."));
        props.insert("subagent_type".into(), json::prop_str("Agent type to use (default \"subagent\")."));
        json::object_schema(Value::Object(props), &["description", "prompt"])
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        Ok(ToolOutput::err("task tool is handled by the runner and should not be called directly"))
    }
}

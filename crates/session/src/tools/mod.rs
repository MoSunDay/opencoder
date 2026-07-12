use std::collections::HashMap;
use std::sync::Arc;

use opencoder_core::ToolArc;
use serde_json::Value;

pub mod bash;
pub mod edit;
pub mod glob;
pub mod grep;
pub mod ls;
pub mod read;
pub mod task;
pub mod write;

pub fn registry() -> HashMap<String, ToolArc> {
    let all: Vec<ToolArc> = vec![
        Arc::new(bash::BashTool) as ToolArc,
        Arc::new(read::ReadTool) as ToolArc,
        Arc::new(write::WriteTool) as ToolArc,
        Arc::new(edit::EditTool) as ToolArc,
        Arc::new(glob::GlobTool) as ToolArc,
        Arc::new(grep::GrepTool) as ToolArc,
        Arc::new(ls::ListTool) as ToolArc,
        Arc::new(task::TaskTool) as ToolArc,
    ];
    all.into_iter().map(|t| (t.name().to_string(), t)).collect()
}

pub fn schema_for(tools: &HashMap<String, ToolArc>) -> Vec<Value> {
    tools
        .values()
        .map(|t| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": t.name(),
                    "description": t.description(),
                    "parameters": opencoder_llm::schema::sanitize_tool_schema(&t.parameters()),
                }
            })
        })
        .collect()
}

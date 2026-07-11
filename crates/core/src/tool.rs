use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub session_id: String,
    pub message_id: String,
    pub agent: String,
    pub working_dir: std::path::PathBuf,
    pub max_output: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn ok(content: impl Into<String>) -> Self {
        ToolOutput {
            content: content.into(),
            is_error: false,
        }
    }
    pub fn err(content: impl Into<String>) -> Self {
        ToolOutput {
            content: content.into(),
            is_error: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value;
    async fn execute(&self, input: Value, ctx: &ToolContext) -> anyhow::Result<ToolOutput>;
}

pub fn schema_of(tool: &dyn Tool) -> ToolSchema {
    ToolSchema {
        name: tool.name().to_string(),
        description: tool.description().to_string(),
        parameters: tool.parameters(),
    }
}

pub type ToolArc = Arc<dyn Tool>;

pub fn truncate_output(content: String, max: usize) -> ToolOutput {
    if content.len() <= max {
        ToolOutput::ok(content)
    } else {
        let preview: String = content.chars().take(max.min(2000)).collect();
        ToolOutput::ok(format!(
            "{preview}\n\n...[output truncated, total {total} bytes]",
            total = content.chars().count()
        ))
    }
}

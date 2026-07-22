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
    /// Outbound proxy URL for capability tools (browser/computer-use). Carries
    /// `config.network.proxy` from the session so tools honor the configured
    /// proxy; env fallbacks are applied at use time via `effective_proxy`.
    pub proxy: Option<String>,
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

/// Maximum number of output lines before truncation.
pub const MAX_OUTPUT_LINES: usize = 800;

/// Maximum output size in bytes before truncation.
pub const MAX_OUTPUT_BYTES: usize = 4096;

/// Truncate tool output to at most [`MAX_OUTPUT_LINES`] lines and
/// `max` bytes (capped by [`MAX_OUTPUT_BYTES`]). When either limit is
/// exceeded the output is cut and a truncation notice is appended.
pub fn truncate_output(content: String, max: usize) -> ToolOutput {
    truncate_output_with_error(content, max, false)
}

/// Like [`truncate_output`] but preserves the `is_error` flag.
pub fn truncate_output_with_error(content: String, max: usize, is_error: bool) -> ToolOutput {
    let max_bytes = max.min(MAX_OUTPUT_BYTES);
    let total_lines = content.lines().count();
    let total_bytes = content.len();

    let over_lines = total_lines > MAX_OUTPUT_LINES;
    let over_bytes = total_bytes > max_bytes;

    if !over_lines && !over_bytes {
        return ToolOutput { content, is_error };
    }

    // Apply line limit first, then byte limit.
    let mut result: String = if over_lines {
        content
            .lines()
            .take(MAX_OUTPUT_LINES)
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        content
    };

    if result.len() > max_bytes {
        let mut end = max_bytes;
        while end > 0 && !result.is_char_boundary(end) {
            end -= 1;
        }
        result.truncate(end);
    }

    let mut parts = Vec::new();
    if over_lines {
        parts.push(format!("{total_lines} lines"));
    }
    if over_bytes {
        parts.push(format!("{total_bytes} bytes"));
    }

    ToolOutput {
        content: format!(
            "{result}\n\n[output truncated, original {}]",
            parts.join(", ")
        ),
        is_error,
    }
}

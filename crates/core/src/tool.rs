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
/// exceeded the output is cut head+tail style and a truncation notice is
/// appended.
pub fn truncate_output(content: String, max: usize) -> ToolOutput {
    truncate_output_with_error(content, max, false)
}

/// Like [`truncate_output`] but preserves the `is_error` flag.
///
/// Truncation keeps a head *and* a tail of the content. Tool output that
/// exceeds the limits often buries its most useful signal — the error
/// message, the failing line, the final exit reason — at the very end, so
/// cutting head-only would discard exactly that. We therefore keep the
/// first and last chunks with an omission marker between them.
pub fn truncate_output_with_error(content: String, max: usize, is_error: bool) -> ToolOutput {
    let max_bytes = max.min(MAX_OUTPUT_BYTES);
    let total_lines = content.lines().count();
    let total_bytes = content.len();

    let over_lines = total_lines > MAX_OUTPUT_LINES;
    let over_bytes = total_bytes > max_bytes;

    if !over_lines && !over_bytes {
        return ToolOutput { content, is_error };
    }

    // Apply the line cap first (head+tail), then the byte cap on the result.
    let after_lines = if over_lines {
        head_tail_lines(&content, MAX_OUTPUT_LINES)
    } else {
        content
    };

    let body = if after_lines.len() > max_bytes {
        head_tail_bytes(&after_lines, max_bytes)
    } else {
        after_lines
    };

    let mut parts = Vec::new();
    if over_lines {
        parts.push(format!("{total_lines} lines"));
    }
    if over_bytes {
        parts.push(format!("{total_bytes} bytes"));
    }

    ToolOutput {
        content: format!("{body}\n\n[output truncated, original {}]", parts.join(", ")),
        is_error,
    }
}

/// Return a head+tail view of `content` holding at most `budget` lines,
/// preserving both the beginning and the end. The dropped middle is summarised
/// by a `[N lines omitted]` marker on its own line. Returns the input
/// unchanged when it already fits; head + marker + tail never exceed `budget`
/// lines.
fn head_tail_lines(content: &str, budget: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= budget {
        return content.to_string();
    }
    // Reserve one line for the omission marker; split the rest head/tail.
    let keep = budget.saturating_sub(1).max(2);
    let head = keep / 2;
    let tail = keep - head;
    let mid = lines.len() - head - tail;
    let mut out = lines[..head].join("\n");
    out.push_str("\n... [");
    out.push_str(&mid.to_string());
    out.push_str(" lines omitted] ...\n");
    out.push_str(&lines[lines.len() - tail..].join("\n"));
    out
}

/// Return a head+tail byte view of `content` holding at most `budget` bytes,
/// preserving the beginning and the end so trailing error text survives. The
/// middle is replaced by a `[N bytes omitted]` marker. Char boundaries are
/// respected on both cuts so the result is always valid UTF-8.
fn head_tail_bytes(content: &str, budget: usize) -> String {
    if content.len() <= budget {
        return content.to_string();
    }
    let head = (budget / 2).max(1);
    let tail = budget.saturating_sub(head).max(1);

    // Walk both cut points onto UTF-8 char boundaries.
    let mut h = head;
    while h > 0 && !content.is_char_boundary(h) {
        h -= 1;
    }
    let end = content.len();
    let mut t_start = end.saturating_sub(tail);
    while t_start < end && !content.is_char_boundary(t_start) {
        t_start += 1;
    }
    let omitted = t_start.saturating_sub(h);

    let mut out = String::with_capacity(h + (end - t_start) + 32);
    out.push_str(&content[..h]);
    out.push_str("\n... [");
    out.push_str(&omitted.to_string());
    out.push_str(" bytes omitted] ...\n");
    out.push_str(&content[t_start..]);
    out
}

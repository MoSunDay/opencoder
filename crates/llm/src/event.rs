use serde::{Deserialize, Serialize};

use crate::tool_call::CompletedToolCall;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone)]
pub enum LlmEvent {
    TextDelta(String),
    ReasoningDelta(String),
    ToolCallStart {
        index: usize,
        id: String,
        name: String,
    },
    ToolCallDelta {
        index: usize,
        arguments: String,
    },
    Completed {
        text: String,
        tool_calls: Vec<CompletedToolCall>,
        usage: Option<Usage>,
    },
    /// Emitted before each retry backoff during the pre-stream retry loop
    /// (`attempt` is 1-based, `max` is the total attempt budget). Lets the UI
    /// surface "↻ retry attempt/max" so a transient failure isn't silent.
    Retrying { attempt: u8, max: u8 },
    Error(String),
}

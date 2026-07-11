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
    Error(String),
}

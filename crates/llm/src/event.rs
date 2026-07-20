use serde::{Deserialize, Serialize};

use crate::tool_call::CompletedToolCall;

/// Token accounting for one LLM turn.
///
/// `input_tokens` / `output_tokens` / `total_tokens` mirror the OpenAI
/// `usage` block (`prompt_tokens` / `completion_tokens` / `total_tokens`).
///
/// `cache_read_tokens` / `cache_creation_tokens` capture prompt-caching
/// accounting. Provider naming is inconsistent, so `parse_usage` normalizes
/// every known variant into these two fields:
///   - Anthropic / most OpenAI-compatible proxies fronting Claude & GLM:
///     `cache_read_input_tokens`, `cache_creation_input_tokens`
///   - Some gateways: `cache_read`, `cache_write`
///   - OpenAI native: nested under `prompt_tokens_details.cached_tokens`
///
/// Persisted verbatim into `messages.usage_json` via `MessageUsage`, so
/// downstream consumers (sync/billing) see the full picture from this turn
/// forward. Historical rows predate these fields and deserialize to `0`
/// (`#[serde(default)]`) -- past cache usage is unrecoverable.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_creation_tokens: u64,
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
    Retrying {
        attempt: u8,
        max: u8,
    },
    Error(String),
}

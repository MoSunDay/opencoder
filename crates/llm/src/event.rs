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
    /// Emitted before each retry backoff (`attempt` is 1-based, `max` is the
    /// total attempt budget). Lets the UI surface "↻ retry attempt/max" so a
    /// transient failure isn't silent.
    ///
    /// Used by BOTH retry loops — the pre-stream connection loop and the
    /// mid-stream interruption loop. When emitted mid-stream, the consumer MUST
    /// discard any deltas accumulated so far (text/reasoning/tool-call
    /// buffers): the retry restarts the response from scratch, so the final
    /// `Completed.text` is a fresh frame, never stitched across attempts.
    /// Persisted text is therefore always internally consistent.
    Retrying {
        attempt: u8,
        max: u8,
    },
    Error(String),
}

use serde_json::{json, Value};

use crate::message::OpenAIMessage;

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<OpenAIMessage>,
    pub tools: Vec<Value>,
    pub tool_choice: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u64>,
    /// OpenAI-style reasoning effort (`low|medium|high`). Forwarded verbatim
    /// as a top-level `reasoning_effort` field on the request body. `None`
    /// omits the field so providers that don't support it stay unaffected.
    pub reasoning_effort: Option<String>,
    /// Per-agent prefix-cache salt. When `Some(non-empty)`, serialized as a
    /// top-level `"cache_salt"` field on the request body so a vLLM /
    /// prefix-cache backend can namespace its KV cache per agent and grow the
    /// cached prefix across turns within a conversation. `None`/empty omits the
    /// field so backends that don't support it stay unaffected.
    pub cache_salt: Option<String>,
}

impl ChatRequest {
    pub fn to_body(&self) -> Value {
        let mut body = json!({
            "model": self.model,
            "messages": self.messages,
            "stream": true,
            "stream_options": { "include_usage": true },
        });
        if !self.tools.is_empty() {
            body["tools"] = json!(self.tools);
            if let Some(tc) = &self.tool_choice {
                body["tool_choice"] = json!(tc);
            }
        }
        if let Some(t) = self.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(m) = self.max_tokens {
            body["max_tokens"] = json!(m);
        }
        if let Some(e) = &self.reasoning_effort {
            let trimmed = e.trim();
            if !trimmed.is_empty() {
                body["reasoning_effort"] = json!(trimmed);
            }
        }
        if let Some(s) = &self.cache_salt {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                body["cache_salt"] = json!(trimmed);
            }
        }
        body
    }
}

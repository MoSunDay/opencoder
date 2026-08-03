use serde_json::{json, Value};

use crate::message::OpenAIMessage;

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<OpenAIMessage>,
    pub tools: Vec<Value>,
    pub tool_choice: Option<String>,
    /// Sampling temperature. Stored as `f64` (not `f32`) so that `json!(t)`
    /// serializes the shortest round-trippable decimal (e.g. `0.3`) rather than
    /// the widened `f32` artifact `0.6999999880790710` that `0.7_f32 as f64`
    /// produces.
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
    /// OpenAI-style reasoning effort (`low|medium|high|xhigh|max`). Forwarded verbatim
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

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_req() -> ChatRequest {
        ChatRequest {
            model: "m".into(),
            messages: Vec::new(),
            tools: Vec::new(),
            tool_choice: None,
            temperature: None,
            max_tokens: None,
            reasoning_effort: None,
            cache_salt: None,
        }
    }

    #[test]
    fn temperature_omitted_when_none() {
        let body = minimal_req().to_body();
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn temperature_serializes_as_clean_f64() {
        // Regression: when the field was `f32`, `0.7_f32` widened to f64 on
        // serialization produced the artifact `0.6999999880790710`. As `f64`,
        // serde_json emits the shortest round-trippable decimal `0.7`.
        let mut req = minimal_req();
        req.temperature = Some(0.7);
        let body = req.to_body();
        // The serialized number must read back exactly as 0.7, not the f32
        // widening artifact.
        assert_eq!(body["temperature"], json!(0.7));
        assert_eq!(body["temperature"].to_string(), "0.7");
    }

    #[test]
    fn temperature_zero_serializes_cleanly() {
        let mut req = minimal_req();
        req.temperature = Some(0.0);
        let body = req.to_body();
        assert_eq!(body["temperature"], json!(0.0));
        assert_eq!(body["temperature"].to_string(), "0.0");
    }
}

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
        body
    }
}

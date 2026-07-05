use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    Reasoning { text: String },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

impl ContentBlock {
    pub fn text(s: impl Into<String>) -> Self {
        ContentBlock::Text { text: s.into() }
    }
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ContentBlock::Text { text } => Some(text),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub role: Role,
    pub blocks: Vec<ContentBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default)]
    pub usage: MessageUsage,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub synthetic: bool,
}

impl Message {
    pub fn user(id: impl Into<String>, text: impl Into<String>) -> Self {
        Message {
            id: id.into(),
            role: Role::User,
            blocks: vec![ContentBlock::text(text)],
            model: None,
            agent: None,
            usage: MessageUsage::default(),
            created_at: now_ms(),
            synthetic: false,
        }
    }
    pub fn assistant(id: impl Into<String>) -> Self {
        Message {
            id: id.into(),
            role: Role::Assistant,
            blocks: vec![],
            model: None,
            agent: None,
            usage: MessageUsage::default(),
            created_at: now_ms(),
            synthetic: false,
        }
    }
    pub fn system(id: impl Into<String>, text: impl Into<String>) -> Self {
        Message {
            id: id.into(),
            role: Role::System,
            blocks: vec![ContentBlock::text(text)],
            model: None,
            agent: None,
            usage: MessageUsage::default(),
            created_at: now_ms(),
            synthetic: false,
        }
    }
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .filter_map(|b| b.as_text())
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

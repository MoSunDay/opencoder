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
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
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
    /// Inline image attached to a user message. `url` is either an
    /// `http(s)://` URL or a `data:image/<fmt>;base64,...` URI. `detail`
    /// maps to the OpenAI `image_url.detail` field (high/low/auto); `None`
    /// leaves the choice to the provider. Excluded from `text()` so the
    /// plain-text view stays clean.
    Image {
        url: String,
        detail: Option<String>,
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
    pub fn as_image(&self) -> Option<(&str, Option<&str>)> {
        match self {
            ContentBlock::Image { url, detail } => Some((url, detail.as_deref())),
            _ => None,
        }
    }
}

/// Persisted token usage for one assistant message, stored in the
/// `messages.usage_json` TEXT column as JSON.
///
/// Mirrors the LLM-layer `Usage`. `cache_read_tokens` /
/// `cache_creation_tokens` carry prompt-cache accounting. `#[serde(default)]`
/// keeps deserialization of pre-cache-tracking rows (which lack these keys)
/// working, yielding `0` for old data -- i.e. historical cache usage cannot
/// be recovered, only tracked from the point this change shipped.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_creation_tokens: u64,
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

    pub fn has_image(&self) -> bool {
        self.blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::Image { .. }))
    }

    /// Build a user message from a text prompt plus zero or more image URIs
    /// (`data:image/...;base64,...` or `http(s)://`). Each image becomes an
    /// `Image` content block appended after the text block. With no images
    /// this is equivalent to [`Message::user`].
    pub fn user_with_images(
        id: impl Into<String>,
        text: impl Into<String>,
        images: &[String],
    ) -> Self {
        let mut blocks = vec![ContentBlock::text(text)];
        for url in images {
            blocks.push(ContentBlock::Image {
                url: url.clone(),
                detail: None,
            });
        }
        Message {
            id: id.into(),
            role: Role::User,
            blocks,
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

    /// Faithful textual rendering of **all** content blocks — Text, Reasoning,
    /// ToolUse input JSON, and ToolResult content — for token estimation.
    /// `text()` only returns `Text` blocks and would undercount an agent-heavy
    /// transcript by 10–50×, breaking compaction thresholds.
    pub fn estimate_chars(&self) -> String {
        let mut out = String::new();
        for block in &self.blocks {
            match block {
                ContentBlock::Text { text } => out.push_str(text),
                ContentBlock::Reasoning { text } => out.push_str(text),
                ContentBlock::ToolUse { name, input, .. } => {
                    out.push_str(name);
                    out.push_str(&serde_json::to_string(input).unwrap_or_default());
                }
                ContentBlock::ToolResult { content, .. } => out.push_str(content),
                // Vision attachments cost ~hundreds of tokens regardless of
                // payload size. Count a fixed rough cost instead of dumping
                // the (huge) base64 URI, which would blow past compaction
                // budgets by orders of magnitude. ~256 tokens per image.
                ContentBlock::Image { .. } => out.push_str(&"x".repeat(1024)),
            }
            out.push('\n');
        }
        out
    }
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

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
        /// Images returned by the tool (data URIs / URLs), forwarded to vision
        /// models as `image_url` parts on the `tool` message. Backward
        /// compatible: persisted rows without this key deserialize to empty.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<String>,
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
    /// Verbatim display text for user input — the echo-side single source of
    /// truth. Populated with the raw prompt (e.g. `$skill` tokens included)
    /// at record time; every display surface (TUI replay, SPA, `session
    /// show`) prefers it over `text()`, which carries the post-resolution
    /// clean text the LLM consumes. Never serialized into LLM wire requests
    /// (those read `blocks` only). `None` on legacy rows: callers fall back
    /// to `text()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
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
            display: None,
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
            display: None,
        }
    }

    /// Like [`Message::user_with_images`] but with a verbatim display text
    /// (the raw user input, `$skill` tokens included). `text` stays the
    /// post-resolution clean text the LLM consumes; `display` is echo-only.
    pub fn user_with_display(
        id: impl Into<String>,
        text: impl Into<String>,
        display: Option<String>,
        images: &[String],
    ) -> Self {
        let mut m = Message::user_with_images(id, text, images);
        m.display = display;
        m
    }

    pub fn assistant(id: impl Into<String>) -> Self {
        Message {
            id: id.into(),
            role: Role::Assistant,
            display: None,
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
            display: None,
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
                ContentBlock::ToolResult {
                    content, images, ..
                } => {
                    out.push_str(content);
                    // Vision attachments returned by a tool cost ~hundreds of
                    // tokens regardless of payload size; count a fixed rough
                    // cost (do NOT dump the base64 URI — it would blow past
                    // compaction budgets). ~256 tokens per image.
                    for _ in images {
                        out.push_str(&"x".repeat(1024));
                    }
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Legacy persisted JSON (predating `display`) deserializes with
    /// `display: None` — no schema/data migration required.
    #[test]
    fn legacy_json_without_display_deserializes_to_none() {
        let raw = r#"{"id":"m1","role":"user","blocks":[{"kind":"text","text":"hi"}]}"#;
        let m: Message = serde_json::from_str(raw).unwrap();
        assert_eq!(m.text(), "hi");
        assert!(m.display.is_none());
        assert!(!m.synthetic);
    }

    /// `display` round-trips through serde and stays absent from the JSON
    /// when `None` (keeps the wire/persisted shape unchanged for old rows).
    #[test]
    fn display_roundtrip_and_skip_when_none() {
        let m =
            Message::user_with_display("m2", "fix the bug", Some("$haiku fix the bug".into()), &[]);
        assert_eq!(m.text(), "fix the bug");
        assert_eq!(m.display.as_deref(), Some("$haiku fix the bug"));

        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"display\":\"$haiku fix the bug\""));
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back.display.as_deref(), Some("$haiku fix the bug"));

        let plain = Message::user("m3", "plain");
        let json = serde_json::to_string(&plain).unwrap();
        assert!(!json.contains("display"), "None display must not serialize");
    }

    /// `user_with_display` with no images and no display is equivalent to
    /// `user`; images land after the text block either way.
    #[test]
    fn user_with_display_mirrors_user_with_images() {
        let a = Message::user_with_display("m4", "t", None, &["img".to_string()]);
        let b = Message::user_with_images("m4", "t", &["img".to_string()]);
        assert_eq!(a.blocks.len(), b.blocks.len());
        assert!(a.display.is_none());
        assert!(a.has_image());
    }
}

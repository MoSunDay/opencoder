use anyhow::Result;
use tokio::sync::mpsc;

use crate::{ChatRequest, LlmEvent};

/// Abstraction over a chat-completion stream. Both the real `ChatClient` and
/// the test `MockChatClient` implement this, so the session runner can be driven
/// deterministically in tests without touching the network.
///
/// `chat_stream` returns immediately; events are produced asynchronously on a
/// background task and delivered through the returned receiver. This mirrors the
/// real streaming HTTP contract (SSE → channel) exactly.
pub trait ChatStream: Send + Sync {
    /// The actual wire body, for recording/debugging wrappers. Configured
    /// clients override this to select the request's provider protocol.
    fn request_body(&self, req: &ChatRequest) -> Result<serde_json::Value> {
        Ok(req.to_body())
    }

    fn chat_stream(&self, req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>>;

    /// Human-readable backend label (e.g. "openai", "mock") for logging/tests.
    fn backend(&self) -> &'static str {
        "chat"
    }

    /// Embed texts via an OpenAI-compatible `/embeddings` endpoint.
    /// Returns one vector per input text, in input order.
    fn embed(&self, _texts: &[String], _model: &str) -> Result<Vec<Vec<f32>>> {
        anyhow::bail!("embeddings are not supported by {}", self.backend())
    }
}

impl<T: ChatStream + ?Sized> ChatStream for std::sync::Arc<T> {
    fn request_body(&self, req: &ChatRequest) -> Result<serde_json::Value> {
        (**self).request_body(req)
    }
    fn chat_stream(&self, req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
        (**self).chat_stream(req)
    }
    fn backend(&self) -> &'static str {
        (**self).backend()
    }
    fn embed(&self, texts: &[String], model: &str) -> Result<Vec<Vec<f32>>> {
        (**self).embed(texts, model)
    }
}

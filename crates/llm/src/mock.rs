//! Deterministic mock `ChatStream` for tests. Records every request and replays
//! scripted event sequences in FIFO order, enabling assertions like
//! "the switched model appears in the next request body".

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use tokio::sync::mpsc;

use crate::{ChatRequest, ChatStream, LlmEvent};

/// One queued `chat_stream` response. `Hang` parks the stream on an external
/// `Notify` so tests can hold an in-flight LLM call open deterministically.
enum ScriptEntry {
    Events(Vec<LlmEvent>),
    Hang(Arc<tokio::sync::Notify>),
}

/// Dimension of every vector produced by the mock embedder. `pub` so upper
/// layers can size their assertions against it.
pub const MOCK_EMBED_DIM: usize = 8;

/// FNV-1a over `bytes`, seeded with `seed` (used to derive one component per
/// dimension). Pure and stable across platforms (wrapping arithmetic only).
fn fnv1a(seed: u32, bytes: &[u8]) -> u32 {
    let mut hash = 0x811c_9dc5u32 ^ seed;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Deterministic unit-length embedding for one text: component `i` is the
/// FNV-1a hash of the text bytes seeded with `i`, mapped into `(0, 1)`; the
/// whole vector is then L2-normalized so cosine similarity is just a dot
/// product. Same text ⇒ same vector; different texts almost surely differ.
fn mock_embedding(text: &str) -> Vec<f32> {
    let mut vec: Vec<f32> = (0..MOCK_EMBED_DIM as u32)
        .map(|i| {
            let h = fnv1a(i, text.as_bytes());
            ((f64::from(h) + 0.5) / (f64::from(u32::MAX) + 1.0)) as f32
        })
        .collect();
    let norm: f32 = vec.iter().map(|c| c * c).sum::<f32>().sqrt();
    if norm > 0.0 {
        for component in &mut vec {
            *component /= norm;
        }
    }
    vec
}

/// Builder-friendly mock. Push one script per expected `chat_stream` call.
pub struct MockChatClient {
    requests: Mutex<Vec<ChatRequest>>,
    scripts: Mutex<VecDeque<ScriptEntry>>,
    default: Mutex<Option<Vec<LlmEvent>>>,
    embed_calls: Mutex<Vec<(Vec<String>, String)>>,
}

impl MockChatClient {
    pub fn new() -> Self {
        MockChatClient {
            requests: Mutex::new(Vec::new()),
            scripts: Mutex::new(VecDeque::new()),
            default: Mutex::new(None),
            embed_calls: Mutex::new(Vec::new()),
        }
    }

    /// Queue the events to return for the next `chat_stream` call (FIFO).
    pub fn push_script(self, events: Vec<LlmEvent>) -> Self {
        self.queue_script(events);
        self
    }

    /// Queue a hanging stream for the next `chat_stream` call: the receiver
    /// yields nothing until `notify` fires (or a permit is stored), then the
    /// channel closes so the stream ends. Lets a test hold an in-flight LLM
    /// call open and release it on demand.
    pub fn push_hang(self, notify: Arc<tokio::sync::Notify>) -> Self {
        self.queue_hang(notify);
        self
    }

    /// Interior-mutable twin of [`push_script`] usable through a shared
    /// `Arc<MockChatClient>` after the handle was handed to a runtime.
    pub fn queue_script(&self, events: Vec<LlmEvent>) {
        self.scripts
            .lock()
            .unwrap()
            .push_back(ScriptEntry::Events(events));
    }

    /// Interior-mutable twin of [`push_hang`] (shared-handle form).
    pub fn queue_hang(&self, notify: Arc<tokio::sync::Notify>) {
        self.scripts
            .lock()
            .unwrap()
            .push_back(ScriptEntry::Hang(notify));
    }

    /// Events returned when no queued script remains. Useful for long loops.
    pub fn with_default(self, events: Vec<LlmEvent>) -> Self {
        *self.default.lock().unwrap() = Some(events);
        self
    }

    /// Snapshot of every request seen, in call order — for contract assertions.
    pub fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Number of `chat_stream` calls observed.
    pub fn call_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    /// Every `embed` call so far as `(texts, model)`, in call order.
    pub fn embed_calls(&self) -> Vec<(Vec<String>, String)> {
        self.embed_calls.lock().unwrap().clone()
    }
}

impl Default for MockChatClient {
    fn default() -> Self {
        MockChatClient::new()
    }
}

impl ChatStream for MockChatClient {
    fn chat_stream(&self, req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
        self.requests.lock().unwrap().push(req);
        let entry = match self.scripts.lock().unwrap().pop_front() {
            Some(entry) => entry,
            None => match self.default.lock().unwrap().clone() {
                Some(events) => ScriptEntry::Events(events),
                None => return Err(anyhow!("mock exhausted: no script queued and no default")),
            },
        };
        let (tx, rx) = mpsc::channel::<LlmEvent>(128);
        tokio::spawn(async move {
            match entry {
                ScriptEntry::Events(events) => {
                    for ev in events {
                        tokio::task::yield_now().await;
                        if tx.send(ev).await.is_err() {
                            break;
                        }
                    }
                }
                ScriptEntry::Hang(notify) => {
                    // Zero events: once released, dropping `tx` closes the
                    // channel and the consumer sees end-of-stream.
                    notify.notified().await;
                }
            }
        });
        Ok(rx)
    }

    fn backend(&self) -> &'static str {
        "mock"
    }

    fn embed(&self, texts: &[String], model: &str) -> Result<Vec<Vec<f32>>> {
        // Deterministic, offline: hash-derived unit vectors in input order.
        self.embed_calls
            .lock()
            .unwrap()
            .push((texts.to_vec(), model.to_string()));
        Ok(texts.iter().map(|t| mock_embedding(t)).collect())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::mpsc::error::TryRecvError;

    use super::*;

    fn req() -> ChatRequest {
        ChatRequest {
            model: "mock-model".into(),
            messages: Vec::new(),
            tools: Vec::new(),
            tool_choice: None,
            temperature: None,
            max_tokens: None,
            reasoning_effort: None,
            cache_salt: None,
        }
    }

    #[tokio::test]
    async fn push_hang_holds_stream_silent_then_ends_after_release() {
        let notify = Arc::new(tokio::sync::Notify::new());
        let mock = MockChatClient::new().push_hang(notify.clone());

        let mut rx = mock.chat_stream(req()).expect("hang must open a stream");
        assert_eq!(mock.call_count(), 1);

        // While hung the channel stays open and silent.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));

        // `notify_one` stores a permit, so the release works even if the
        // spawned hang task has not yet polled `notified()`.
        notify.notify_one();

        let end = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("stream must end shortly after release");
        assert!(end.is_none(), "hang sends zero events, then closes");

        // The hang entry is consumed exactly once; the queue is now empty.
        assert!(mock.chat_stream(req()).is_err());
    }

    // ---- deterministic embeddings ----

    fn norm(v: &[f32]) -> f32 {
        v.iter().map(|c| c * c).sum::<f32>().sqrt()
    }

    #[test]
    fn embed_is_deterministic_and_unit_length() {
        let mock = MockChatClient::new();
        let texts = vec!["hello world".to_string(), "other".to_string()];
        let a = mock.embed(&texts, "text-embedding-3-small").unwrap();
        let b = mock.embed(&texts, "text-embedding-3-small").unwrap();
        // Same texts ⇒ identical vectors, one per input, right dimension.
        assert_eq!(a.len(), 2);
        assert_eq!(a, b);
        assert!(a.iter().all(|v| v.len() == MOCK_EMBED_DIM));
        // Unit L2 norm ⇒ cosine semantics work via plain dot products.
        for v in &a {
            assert!((norm(v) - 1.0).abs() < 1e-5, "norm was {}", norm(v));
            assert!(v.iter().all(|c| *c > 0.0), "components must be in (0,1]");
        }
    }

    #[test]
    fn embed_distinguishes_different_texts() {
        let mock = MockChatClient::new();
        let a = mock
            .embed(&["alpha".to_string()][..], "m")
            .unwrap()
            .remove(0);
        let b = mock
            .embed(&["beta".to_string()][..], "m")
            .unwrap()
            .remove(0);
        assert_ne!(a, b);
    }

    #[test]
    fn embed_records_calls_with_model() {
        let mock = MockChatClient::new();
        let texts = vec!["one".to_string(), "two".to_string()];
        mock.embed(&texts, "text-embedding-3-large").unwrap();
        let calls = mock.embed_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, texts);
        assert_eq!(calls[0].1, "text-embedding-3-large");
        // embed never touches the chat script queue.
        assert_eq!(mock.call_count(), 0);
    }

    #[test]
    fn embed_forwards_through_arc() {
        let mock = Arc::new(MockChatClient::new());
        let stream: std::sync::Arc<MockChatClient> = mock.clone();
        let _ = ChatStream::embed(&stream, &["x".into()][..], "m").unwrap();
        assert_eq!(mock.embed_calls().len(), 1);
    }
}

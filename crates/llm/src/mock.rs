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

/// Builder-friendly mock. Push one script per expected `chat_stream` call.
pub struct MockChatClient {
    requests: Mutex<Vec<ChatRequest>>,
    scripts: Mutex<VecDeque<ScriptEntry>>,
    default: Mutex<Option<Vec<LlmEvent>>>,
}

impl MockChatClient {
    pub fn new() -> Self {
        MockChatClient {
            requests: Mutex::new(Vec::new()),
            scripts: Mutex::new(VecDeque::new()),
            default: Mutex::new(None),
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
}

//! Deterministic mock `ChatStream` for tests. Records every request and replays
//! scripted event sequences in FIFO order, enabling assertions like
//! "the switched model appears in the next request body".

use std::collections::VecDeque;
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use tokio::sync::mpsc;

use crate::{ChatRequest, ChatStream, LlmEvent};

/// Builder-friendly mock. Push one script per expected `chat_stream` call.
pub struct MockChatClient {
    requests: Mutex<Vec<ChatRequest>>,
    scripts: Mutex<VecDeque<Vec<LlmEvent>>>,
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
        self.scripts.lock().unwrap().push_back(events);
        self
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
        let script = match self.scripts.lock().unwrap().pop_front() {
            Some(s) => s,
            None => match self.default.lock().unwrap().clone() {
                Some(s) => s,
                None => return Err(anyhow!("mock exhausted: no script queued and no default")),
            },
        };
        let (tx, rx) = mpsc::channel::<LlmEvent>(128);
        tokio::spawn(async move {
            for ev in script {
                tokio::task::yield_now().await;
                if tx.send(ev).await.is_err() {
                    break;
                }
            }
        });
        Ok(rx)
    }

    fn backend(&self) -> &'static str {
        "mock"
    }
}

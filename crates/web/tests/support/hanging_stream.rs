//! Shared never-yielding `ChatStream` test double for integration tests.
//!
//! [`HangingStream`] counts calls and then parks forever: the spawned task
//! holds the sender and never resolves, so the receiver yields neither an
//! event nor a close, keeping a drain deterministically "running" inside its
//! first LLM turn until a cancel token wins.

#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};

use opencoder_llm::{ChatRequest, ChatStream, LlmEvent};
use tokio::sync::mpsc;

pub struct HangingStream {
    pub calls: AtomicUsize,
}

impl HangingStream {
    pub fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl Default for HangingStream {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatStream for HangingStream {
    fn chat_stream(&self, _req: ChatRequest) -> anyhow::Result<mpsc::Receiver<LlmEvent>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel::<LlmEvent>(128);
        tokio::spawn(async move {
            std::future::pending::<()>().await;
            drop(tx);
        });
        Ok(rx)
    }
}

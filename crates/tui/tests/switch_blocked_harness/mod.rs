//! Shared harness for the `switch_blocked_while_running.rs` integration
//! tests: mock constructors plus the async poll helpers that drive the SAME
//! single-threaded FIFO worker loop the real TUI spawns (MockChatClient +
//! the real `process_cmd`). Split out to keep the contract file under the
//! 400-line new-file cap.

use std::time::Duration;

use opencoder_core::{ContentBlock, Message};
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_session::SessionState;
use opencoder_tui::worker::{process_cmd, UiCmd, UiEvent};
use tokio::sync::mpsc;

pub fn assistant_with_text(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m
}

pub fn text_done(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

/// Spawn the same single-threaded FIFO worker loop the real TUI runs. The
/// returned task yields the final `SessionState` after `UiCmd::Quit`.
pub async fn spawn_worker(
    sess: SessionState,
) -> (
    mpsc::Sender<UiCmd>,
    mpsc::Receiver<UiEvent>,
    tokio::task::JoinHandle<SessionState>,
) {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<UiCmd>(64);
    let (evt_tx, evt_rx) = mpsc::channel::<UiEvent>(512);
    let handle = tokio::spawn(async move {
        let mut sess = sess;
        while let Some(cmd) = cmd_rx.recv().await {
            if process_cmd(cmd, &mut sess, &evt_tx).await {
                break;
            }
        }
        sess
    });
    (cmd_tx, evt_rx, handle)
}

/// Poll until the mock has observed `n` `chat_stream` calls (an in-flight or
/// settled turn's observable footprint).
pub async fn wait_for_calls(mock: &MockChatClient, n: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while mock.call_count() < n {
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "timed out waiting for {n} mock calls, got {}",
                mock.call_count()
            );
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Drain buffered events, then poll until `pred` matches the accumulated
/// batch (or panic on timeout). Returns everything seen so far.
pub async fn wait_for_events<F>(
    rx: &mut mpsc::Receiver<UiEvent>,
    mut pred: F,
    what: &str,
) -> Vec<UiEvent>
where
    F: FnMut(&[UiEvent]) -> bool,
{
    let mut seen: Vec<UiEvent> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        while let Ok(ev) = rx.try_recv() {
            seen.push(ev);
        }
        if pred(&seen) {
            return seen;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for {what}; saw {} events", seen.len());
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

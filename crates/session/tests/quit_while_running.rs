//! Verifies the mechanism the TUI Ctrl+D quit fix relies on: cancelling a
//! session while its LLM stream is *hung* (open but never delivering) must
//! make `run` return promptly via the runner's biased `select!` cancel arm,
//! instead of blocking until the request times out.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent};
use opencoder_session::{run, SessionEvent, SessionState};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// A `ChatStream` that opens a channel but never sends any event and never
/// closes it — a hung LLM connection. The senders are retained for the life of
/// the mock so `rx.recv()` stays pending forever.
struct HungStream {
    keep_alive: Mutex<Vec<mpsc::Sender<LlmEvent>>>,
}

impl HungStream {
    fn new() -> Self {
        HungStream {
            keep_alive: Mutex::new(Vec::new()),
        }
    }
}

impl ChatStream for HungStream {
    fn chat_stream(&self, _req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
        let (tx, rx) = mpsc::channel::<LlmEvent>(8);
        self.keep_alive.lock().unwrap().push(tx);
        Ok(rx)
    }
    fn backend(&self) -> &'static str {
        "hung"
    }
}

#[tokio::test]
async fn cancel_unblocks_a_hung_stream_promptly() {
    let mock = Arc::new(HungStream::new()) as Arc<dyn ChatStream>;
    let config = Config {
        model: "main/glm-5.2".into(),
        ..Config::default()
    };
    let agent = resolve_agent("act").unwrap();
    let cancel = CancellationToken::new();
    let mut s = SessionState::new("quit-run", agent, config, mock, std::env::temp_dir())
        .with_cancel(cancel.clone());

    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let ev_clone = events.clone();

    let start = Instant::now();
    let handle = tokio::spawn(async move {
        run(&mut s, "go".into(), move |ev| {
            ev_clone.lock().unwrap().push(ev);
        })
        .await
    });

    // Let the LLM call start and the stream select! loop engage, then cancel
    // (mimicking Ctrl+D while a turn is running).
    tokio::time::sleep(Duration::from_millis(300)).await;
    cancel.cancel();

    // If the cancel arm works, run returns well under the outer bound. If it
    // does not, run blocks forever and the 5s timeout catches it.
    let outcome = tokio::time::timeout(Duration::from_secs(5), handle).await;
    let elapsed = start.elapsed();

    assert!(
        outcome.is_ok(),
        "run did not return within 5s; cancel does not unblock a hung stream"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "run took {elapsed:?}; expected a sub-2s abort"
    );

    let saw_interrupted = events
        .lock()
        .unwrap()
        .iter()
        .any(|ev| matches!(ev, SessionEvent::Status(msg) if msg == "interrupted"));
    assert!(
        saw_interrupted,
        "expected a Status(interrupted) event after cancel"
    );
}

//! D2 regression, REVISED contract: a hard cancel (web /stop, double-Esc)
//! that arrives while an LLM stream is mid-flight must NOT persist an empty
//! assistant message.
//!
//! D2 originally also forbade `Done` on this path because a consumer treated
//! `Done` as "clean finish". The event semantics have since been redefined
//! (real-browser acceptance of the fleet console): `Done` is the TERMINAL
//! FRAME that closes the SSE stream — without it the web console stays busy
//! forever after an interrupt — while `Status("interrupted")` carries the
//! human-visible reason. Every interrupt exit therefore emits both. The
//! invariants that survive from D2: the interrupted run must stop promptly
//! and must never record an empty assistant message.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use opencoder_core::{resolve_agent, Config, ContentBlock, Role};
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent};
use opencoder_session::{run, SessionEvent, SessionState};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

/// A stream whose first `chat_stream` call never resolves (held alive, never
/// sends), simulating a long-running LLM response so a hard cancel can fire
/// mid-stream.
struct BlockingFirstStream {
    calls: AtomicUsize,
}

impl BlockingFirstStream {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl ChatStream for BlockingFirstStream {
    fn chat_stream(&self, _req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel::<LlmEvent>(128);
        if n == 0 {
            // First call: never send anything. The sender is held alive so
            // `rx.recv()` never resolves; the hard cancel wins the biased
            // select inside the LLM stream loop.
            tokio::spawn(async move {
                std::future::pending::<()>().await;
                drop(tx);
            });
        } else {
            // Any later call: never expected in this test, but stay pending so
            // a regression can't silently complete with a Done.
            tokio::spawn(async move {
                std::future::pending::<()>().await;
                drop(tx);
            });
        }
        Ok(rx)
    }
}

#[tokio::test]
async fn hard_cancel_midstream_no_empty_assistant() {
    let mock = Arc::new(BlockingFirstStream::new()) as Arc<dyn ChatStream>;
    let agent = resolve_agent("act").unwrap();
    let cancel = CancellationToken::new();
    let cancel_for_wait = cancel.clone();
    let mut session = SessionState::new(
        "d2-hard-cancel-midstream",
        agent,
        config(),
        mock,
        std::env::temp_dir(),
    )
    .with_cancel(cancel.clone());

    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();

    let waiter = tokio::spawn(async move {
        // Let the runner enter the (blocking) chat_stream call, then fire the
        // hard cancel mid-stream.
        tokio::time::sleep(Duration::from_millis(300)).await;
        cancel_for_wait.cancel();
    });

    let start = std::time::Instant::now();
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        run(&mut session, "go".into(), move |ev| {
            events_clone.lock().unwrap().push(ev);
        }),
    )
    .await;
    let _ = waiter.await;
    let elapsed = start.elapsed();

    assert!(
        outcome.is_ok(),
        "run did not return within 10s; hard cancel mid-stream did not stop the loop"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "run took {elapsed:?}; expected a prompt break on hard cancel"
    );

    let evs = events.lock().unwrap();

    // The interrupted status must have been emitted by run_one_llm_call.
    let saw_interrupted = evs
        .iter()
        .any(|ev| matches!(ev, SessionEvent::Status(msg) if msg == "interrupted"));
    assert!(
        saw_interrupted,
        "expected a Status(interrupted) event after hard cancel"
    );

    // Terminal frame: `Done` closes the SSE stream; without it the web
    // console stays busy forever after the interrupt (real-browser
    // acceptance). `Status("interrupted")` above carries the reason.
    let saw_done = evs.iter().any(|ev| matches!(ev, SessionEvent::Done));
    assert!(
        saw_done,
        "hard-cancel mid-stream must still emit the terminal Done frame"
    );

    drop(evs);

    // No empty assistant message may be persisted. Before the fix the empty
    // turn produced an assistant message with NO content blocks (no text,
    // reasoning, or tool calls). After the fix the hard-cancel guard breaks
    // before any assistant message is recorded.
    let empty_assistant = session
        .messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .find(|m| {
            m.blocks
                .iter()
                .all(|b| matches!(b, ContentBlock::Text { text } if text.trim().is_empty()))
                || m.blocks.is_empty()
        });
    assert!(
        empty_assistant.is_none(),
        "hard-cancel mid-stream must not persist an empty assistant message (D2 bug); \
         messages: {:?}",
        session
            .messages
            .iter()
            .map(|m| (m.role, m.blocks.len()))
            .collect::<Vec<_>>()
    );

    // Stronger: since the only LLM call was interrupted before completing,
    // there must be NO assistant message at all.
    let any_assistant = session.messages.iter().any(|m| m.role == Role::Assistant);
    assert!(
        !any_assistant,
        "an interrupted LLM stream must leave no assistant message behind; got {} assistant msgs",
        session
            .messages
            .iter()
            .filter(|m| m.role == Role::Assistant)
            .count()
    );
}

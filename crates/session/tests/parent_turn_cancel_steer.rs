//! G1 regression: a parent `>` steer with NO running children must interrupt
//! the parent's current LLM turn via `turn_cancel` (a soft turn-level
//! interrupt), NOT hard-abort via `cancel.cancel()`.
//!
//! Before the fix, the TUI's SteerSubmit handler fell through to
//! `cancel.cancel()` when there were no children to cancel, which broke the
//! `run_loop` entirely (session died). After the fix, `fire_turn_cancel` fires
//! the separate `turn_cancel` token: `await_turn_cancel` wins the biased
//! `select!` inside the LLM stream loop, the call returns an empty turn,
//! `run_loop` detects `is_turn_cancelled`, resets the token, and continues to
//! the next iteration where `claim_steers` absorbs the pending steer.
//!
//! This test verifies the runtime contract the G1 fix relies on: firing
//! `turn_cancel` interrupts the in-flight LLM call, keeps the parent's `cancel`
//! intact, and lets `run_loop` continue to absorb the steer.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, Usage};
use opencoder_session::{fire_turn_cancel, run, SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, SessionInput, SessionMeta, Store};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn text_done(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: Some(Usage {
            input_tokens: 5,
            output_tokens: 5,
            total_tokens: 10,
            ..Default::default()
        }),
    }
}

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

/// A ChatStream whose first `chat_stream` call blocks forever (never produces
/// events). This simulates a long-running LLM response so we can fire
/// `turn_cancel` mid-stream. Subsequent calls return `text_done` immediately.
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
            // First call: never send anything. The sender is held alive by the
            // pending task so `rx.recv()` never resolves; `turn_cancel` wins
            // the biased select inside the LLM stream loop.
            tokio::spawn(async move {
                std::future::pending::<()>().await;
                drop(tx);
            });
        } else {
            tokio::spawn(async move {
                let _ = tx.send(text_done("recovered after steer")).await;
            });
        }
        Ok(rx)
    }
}

#[tokio::test]
async fn turn_cancel_interrupts_llm_without_hard_abort() {
    let store = mem_store().await;
    let mock = Arc::new(BlockingFirstStream::new()) as Arc<dyn ChatStream>;

    let agent = resolve_agent("act").unwrap();
    let cancel = CancellationToken::new();
    let session_id = "parent-turn-cancel-g1-1".to_string();
    let session = SessionState::new(
        session_id.clone(),
        agent,
        config(),
        mock,
        std::env::temp_dir(),
    )
    .with_store(store.clone())
    .with_cancel(cancel.clone());

    // Grab the turn_cancel token before moving the session into the spawn.
    let turn_cancel = session
        .turn_cancel
        .clone()
        .expect("SessionState::new must default turn_cancel to Some");

    // The session_inputs table has a FK on session_id, so create the session
    // row before admitting the steer.
    store
        .create_session(&SessionMeta {
            id: session_id.clone(),
            title: None,
            agent: Some("act".into()),
            model: Some("m/g".into()),
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
        })
        .await
        .unwrap();
    // Pre-admit a steer input so claim_steers can absorb it after the interrupt.
    let steer = SessionInput {
        seq: None,
        id: "steer-1".into(),
        session_id: session_id.clone(),
        delivery: opencoder_store::Delivery::Steer,
        prompt: "new direction from steer".into(),
        images: Vec::new(),
        display_text: None,
        admitted_seq: 0,
        promoted_seq: None,
    };
    store.admit_input(&steer).await.unwrap();

    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_clone = events.clone();

    let mut session = session;
    let handle = tokio::spawn(async move {
        run(&mut session, "go".into(), move |ev| {
            events_clone.lock().unwrap().push(ev);
        })
        .await
    });

    // Give the runner time to enter the first (blocking) chat_stream call.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Fire the turn-level interrupt — this is what the TUI SteerSubmit handler
    // now does instead of cancel.cancel().
    fire_turn_cancel(&turn_cancel);

    // Must finish quickly (not wait for the blocking stream or idle timeout).
    let result = tokio::time::timeout(Duration::from_secs(10), handle).await;
    assert!(
        result.is_ok(),
        "run did not complete within 10s; turn_cancel did not interrupt the LLM stream"
    );

    // The parent's own cancel must NOT have been fired (no hard abort).
    assert!(
        !cancel.is_cancelled(),
        "parent cancel must remain intact — turn_cancel is a soft interrupt, not a hard abort"
    );

    {
        let evs = events.lock().unwrap();

        // The steer must have been consumed at the next turn boundary.
        let steer_consumed = evs
            .iter()
            .any(|e| matches!(e, SessionEvent::SteerConsumed { .. }));
        assert!(
            steer_consumed,
            "expected SteerConsumed after turn_cancel (the steer was absorbed at the next boundary)"
        );

        // The run must have produced the "recovered" text from the second call.
        let done = evs.iter().any(|e| matches!(e, SessionEvent::Done));
        assert!(done, "expected Done event — run must complete normally");
    }
}

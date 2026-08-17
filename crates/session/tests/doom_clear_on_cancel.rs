//! Bug 13 regression: a mid-run turn-cancel must clear the doom-loop signature
//! deque so stale pre-cancel tool repetitions do not false-trip the doom guard
//! (`DOOM_THRESHOLD = 20`) after the loop resumes.
//!
//! The doom deque is local to `run_loop` and not directly observable, so this
//! is a differential test over one `run` call. A custom `ChatStream` returns
//! `K_PRE` identical `bash` turns, fires `turn_cancel` on the next call, then
//! `M_POST` more identical `bash` turns, then a text-only "done" turn.
//!
//! `K_PRE < DOOM_THRESHOLD` but `K_PRE + M_POST > DOOM_THRESHOLD` with
//! `M_POST < DOOM_THRESHOLD`:
//!   - With the fix (`doom.clear()` on cancel): only `M_POST` fresh signatures
//!     accumulate post-cancel -> no trip -> `run` returns `Ok` + `Done`.
//!   - Without the fix: the `K_PRE` stale signatures survive the cancel, so the
//!     guard trips after only `DOOM_THRESHOLD - K_PRE` post-cancel turns ->
//!     `run` returns `Err` + a "doom-loop" error event.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatRequest, ChatStream, CompletedToolCall, LlmEvent, Usage};
use opencoder_session::{fire_turn_cancel, run, SessionEvent, SessionState};
use serde_json::json;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Identical `bash` turns before the cancel; must satisfy `0 < K_PRE < 20`.
const K_PRE: usize = 15;
/// Identical `bash` turns after the cancel; must satisfy
/// `M_POST < 20` and `K_PRE + M_POST > 20` so the buggy path would trip.
const M_POST: usize = 10;
/// 0-based index of the `chat_stream` call that fires `turn_cancel`.
const CANCEL_AT: usize = K_PRE;
/// 0-based index of the final text-only turn ending the loop.
const DONE_AT: usize = K_PRE + 1 + M_POST;

fn bash_turn(i: usize) -> LlmEvent {
    LlmEvent::Completed {
        text: String::new(),
        tool_calls: vec![CompletedToolCall {
            id: format!("tu{i}"),
            name: "bash".into(),
            input: json!({ "command": "true" }),
        }],
        usage: Some(Usage::default()),
    }
}

fn done_turn() -> LlmEvent {
    LlmEvent::Completed {
        text: "done".into(),
        tool_calls: vec![],
        usage: Some(Usage::default()),
    }
}

/// Deterministic `ChatStream` that self-fires `turn_cancel` on the
/// `CANCEL_AT`-th call — inside the sync `chat_stream`, before `run_one_llm_call`
/// reaches its biased `select!`. Because the token is already cancelled when the
/// select first polls `await_turn_cancel`, that arm wins and the call returns an
/// empty turn (no tool calls, so `run_loop` clears doom and continues). No
/// external timer is required, making the test timing-independent.
struct CancelOnKStream {
    calls: AtomicUsize,
    turn_cancel: Arc<Mutex<CancellationToken>>,
}

impl CancelOnKStream {
    fn new(turn_cancel: Arc<Mutex<CancellationToken>>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            turn_cancel,
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ChatStream for CancelOnKStream {
    fn chat_stream(&self, _req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
        let i = self.calls.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel::<LlmEvent>(128);
        if i == CANCEL_AT {
            // Fire the turn-level interrupt synchronously. The biased select
            // resolves the turn_cancel arm first; the receiver below is never
            // read (no sender is spawned), which is harmless.
            fire_turn_cancel(&self.turn_cancel);
            return Ok(rx);
        }
        let ev = if i == DONE_AT {
            done_turn()
        } else {
            bash_turn(i)
        };
        tokio::spawn(async move {
            let _ = tx.send(ev).await;
        });
        Ok(rx)
    }
}

#[tokio::test]
async fn turn_cancel_clears_doom_signatures() {
    let turn_cancel: Arc<Mutex<CancellationToken>> = Arc::new(Mutex::new(CancellationToken::new()));
    let stream = Arc::new(CancelOnKStream::new(turn_cancel.clone()));
    let stream_for_count = Arc::clone(&stream);

    let mut session = SessionState::new(
        "doom-clear-on-cancel".to_string(),
        resolve_agent("act").unwrap(),
        Config {
            model: "m/g".into(),
            ..Config::default()
        },
        stream,
        std::env::temp_dir(),
    )
    .with_turn_cancel(turn_cancel.clone());

    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let ev_clone = events.clone();

    let result = run(&mut session, "go".into(), move |ev: SessionEvent| {
        ev_clone.lock().unwrap().push(ev);
    })
    .await;

    let guard = events.lock().unwrap();
    let has_done = guard.iter().any(|e| matches!(e, SessionEvent::Done));
    let doom_error = guard
        .iter()
        .any(|e| matches!(e, SessionEvent::Error(msg) if msg.contains("doom-loop")));

    // With the fix: all M_POST post-cancel turns run without tripping the guard.
    assert!(
        result.is_ok(),
        "run should succeed after cancel cleared doom, got {result:?}"
    );
    assert!(has_done, "expected a Done event; events = {guard:?}");
    assert!(
        !doom_error,
        "doom-loop must not trip after cancel cleared signatures; events = {guard:?}"
    );
    // Every scripted turn was driven, including the interrupted one (which
    // still consumed a chat_stream call).
    assert_eq!(
        stream_for_count.call_count(),
        DONE_AT + 1,
        "expected {} chat_stream calls",
        DONE_AT + 1
    );
}

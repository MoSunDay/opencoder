//! Interrupt must end the event stream with a terminal `Done` frame.
//!
//! Found by real-browser acceptance (fleet console): POST /interrupt stopped
//! the drain, but the runner's interrupt exits emitted only
//! `Status("interrupted")` — never a terminal frame. The SSE stream stayed
//! open, so the web console hung in `streaming…` forever (busy=true: 发送
//! disabled, 中断 enabled) until a manual reload. This pins BOTH interrupt
//! exits: the loop-head cancel check, the mid-turn tool-exec check, the
//! mid-LLM-round check (cancel while the model is still streaming), and the
//! hard-cancel exit in apply_steer_batch.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, Store};
use tokio_util::sync::CancellationToken;

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn bash_call(cmd: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: String::new(),
        tool_calls: vec![CompletedToolCall {
            id: "bash-1".into(),
            name: "bash".into(),
            input: serde_json::json!({ "command": cmd }),
        }],
        usage: Some(Usage {
            input_tokens: 5,
            output_tokens: 5,
            total_tokens: 10,
            ..Default::default()
        }),
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

/// Every terminated run must end with a terminal frame — `Done` is the last
/// event, and it comes after the human-readable `interrupted` status.
fn assert_terminal_done(evs: &[SessionEvent]) {
    let interrupted = evs
        .iter()
        .any(|e| matches!(e, SessionEvent::Status(m) if m == "interrupted"));
    assert!(interrupted, "Status(\"interrupted\") must be emitted");
    assert!(
        matches!(evs.last(), Some(SessionEvent::Done)),
        "the run must end with a terminal Done frame, got {:?}",
        evs.last()
    );
}

/// A chat backend whose stream stalls (no events) for `stall_ms` before
/// completing — lets the test cancel while the LLM round is still in flight.
struct StalledChat {
    stall_ms: u64,
}

impl ChatStream for StalledChat {
    fn chat_stream(
        &self,
        _req: opencoder_llm::ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let stall = std::time::Duration::from_millis(self.stall_ms);
        let done = text_done("late");
        tokio::spawn(async move {
            tokio::time::sleep(stall).await;
            let _ = tx.send(done).await;
        });
        Ok(rx)
    }
    fn backend(&self) -> &'static str {
        "stalled-test"
    }
}

/// Exit 1: the cancel token is already set when the loop is entered.
#[tokio::test]
async fn loop_head_interrupt_emits_done() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    let mock = Arc::new(MockChatClient::new().with_default(vec![text_done("never")]))
        as Arc<dyn ChatStream>;
    let agent = resolve_agent("act").unwrap();
    let mut s = SessionState::new(
        "interrupt-loop-head",
        agent,
        config(),
        mock,
        std::env::temp_dir(),
    )
    .with_cancel(cancel);
    let mut evs: Vec<SessionEvent> = Vec::new();
    run(&mut s, "go".into(), |ev| evs.push(ev)).await.unwrap();
    assert_terminal_done(&evs);
}

/// Exit 2: cancel fires while a tool is executing mid-turn.
#[tokio::test]
async fn tool_exec_interrupt_emits_done() {
    let store = mem_store().await;
    let mock = Arc::new(
        MockChatClient::new()
            .push_script(vec![bash_call("sleep 30")])
            .push_script(vec![text_done("never")]),
    ) as Arc<dyn ChatStream>;
    let agent = resolve_agent("act").unwrap();
    let cancel = CancellationToken::new();
    let mut session = SessionState::new(
        "interrupt-tool-exec",
        agent,
        config(),
        mock,
        std::env::temp_dir(),
    )
    .with_cancel(cancel.clone())
    .with_store(store);
    let evs: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let evs_clone = evs.clone();
    let handle = tokio::spawn(async move {
        run(&mut session, "go".into(), move |ev| {
            evs_clone.lock().unwrap().push(ev);
        })
        .await
    });

    // let the bash tool start, then cancel mid-execution
    tokio::time::sleep(Duration::from_millis(500)).await;
    cancel.cancel();

    let result = tokio::time::timeout(Duration::from_secs(15), handle).await;
    assert!(
        result.is_ok(),
        "run did not complete within 15s after interrupt"
    );
    assert_terminal_done(&evs.lock().unwrap());
}

/// Exit 3: cancel fires while the LLM round itself is still streaming —
/// run_one_llm_call emits Status("interrupted") and returns an empty turn;
/// the run loop's hard-cancel break must still emit the terminal Done frame.
#[tokio::test]
async fn llm_round_interrupt_emits_done() {
    let mock = Arc::new(StalledChat { stall_ms: 10_000 }) as Arc<dyn ChatStream>;
    let agent = resolve_agent("act").unwrap();
    let cancel = CancellationToken::new();
    let mut session = SessionState::new(
        "interrupt-llm-round",
        agent,
        config(),
        mock,
        std::env::temp_dir(),
    )
    .with_cancel(cancel.clone());
    let evs: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let evs_clone = evs.clone();
    let handle = tokio::spawn(async move {
        run(&mut session, "go".into(), move |ev| {
            evs_clone.lock().unwrap().push(ev);
        })
        .await
    });

    // cancel while the stalled LLM round is in flight
    tokio::time::sleep(Duration::from_millis(300)).await;
    cancel.cancel();

    let result = tokio::time::timeout(Duration::from_secs(15), handle).await;
    assert!(
        result.is_ok(),
        "run did not complete within 15s after interrupt"
    );
    assert_terminal_done(&evs.lock().unwrap());
}

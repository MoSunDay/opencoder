//! Mid-turn `/model` picks. A frontend queues `(mode, model)` while a request
//! is in flight; the runner applies it when that request completes, so the
//! in-flight request keeps the model the status bar showed and the next
//! request (later round of the same turn, or the next turn) uses the new one.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use opencoder_core::{resolve_agent, Config, ConfigMode};
use opencoder_llm::{ChatRequest, ChatStream, CompletedToolCall, LlmEvent, MockChatClient};
use opencoder_session::{
    pending_model_switches, queue_model_switch, run, SessionEvent, SessionState,
};
use opencoder_store::{SessionMeta, Store, TursoStore};
use tokio::sync::{mpsc, Notify};

fn config() -> Config {
    Config {
        model: "provider/before".into(),
        ..Config::default()
    }
}

fn done() -> LlmEvent {
    LlmEvent::Completed {
        text: "done".into(),
        tool_calls: vec![],
        usage: None,
    }
}

/// The first `chat_stream` call announces itself, waits on `gate`, then
/// completes with a `bash` tool call (so the same turn needs a second round);
/// every later call completes immediately with a final answer.
struct GatedToolRound {
    started: Notify,
    gate: Arc<Notify>,
    calls: AtomicUsize,
    requests: Mutex<Vec<ChatRequest>>,
}

impl ChatStream for GatedToolRound {
    fn chat_stream(&self, req: ChatRequest) -> anyhow::Result<mpsc::Receiver<LlmEvent>> {
        self.requests.lock().unwrap().push(req);
        let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        let gate = self.gate.clone();
        let (tx, rx) = mpsc::channel(128);
        if first {
            self.started.notify_one();
        }
        tokio::spawn(async move {
            let event = if first {
                gate.notified().await;
                LlmEvent::Completed {
                    text: String::new(),
                    tool_calls: vec![CompletedToolCall {
                        id: "t1".into(),
                        name: "bash".into(),
                        input: serde_json::json!({ "command": "printf mid-turn" }),
                    }],
                    usage: None,
                }
            } else {
                LlmEvent::Completed {
                    text: "done".into(),
                    tool_calls: vec![],
                    usage: None,
                }
            };
            let _ = tx.send(event).await;
        });
        Ok(rx)
    }

    fn backend(&self) -> &'static str {
        "gated-mock"
    }
}

fn position(events: &[SessionEvent], pred: impl Fn(&SessionEvent) -> bool) -> usize {
    events
        .iter()
        .position(pred)
        .expect("expected event present")
}

#[tokio::test]
async fn pick_queued_during_a_request_applies_after_it_and_before_the_next_one() {
    let client = Arc::new(GatedToolRound {
        started: Notify::new(),
        gate: Arc::new(Notify::new()),
        calls: AtomicUsize::new(0),
        requests: Mutex::new(Vec::new()),
    });
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "mid-turn-model-pick",
        resolve_agent("act").unwrap(),
        config(),
        client.clone(),
        dir.path().to_path_buf(),
    );
    let queue = session.model_switch_queue.clone();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);

    let run_task = tokio::spawn(async move {
        run(&mut session, "go".into(), move |event| {
            sink.lock().unwrap().push(event);
        })
        .await
        .unwrap();
        session
    });
    tokio::time::timeout(Duration::from_secs(5), client.started.notified())
        .await
        .expect("the first request never started");

    // The pick lands while the first request is still streaming.
    queue_model_switch(&queue, ConfigMode::Act, "provider/after");
    client.gate.notify_one();
    let session = run_task.await.unwrap();
    let events = Arc::try_unwrap(events).unwrap().into_inner().unwrap();

    let requests = client.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2, "tool round then final answer");
    assert_eq!(
        requests[0].model, "provider/before",
        "the in-flight request keeps its model"
    );
    assert_eq!(
        requests[1].model, "provider/after",
        "the next request of the same turn uses the pick"
    );

    let first_round = position(&events, |e| matches!(e, SessionEvent::LlmRoundStart { .. }));
    let second_round = events
        .iter()
        .rposition(|e| matches!(e, SessionEvent::LlmRoundStart { .. }))
        .unwrap();
    let switch = position(
        &events,
        |e| matches!(e, SessionEvent::ModelSwitch(m) if m == "after"),
    );
    let applied = position(&events, |e| {
        matches!(
            e,
            SessionEvent::ModelOverrideApplied { mode, model, error: None }
                if mode == "act" && model == "provider/after"
        )
    });
    assert!(
        first_round < applied && applied < second_round,
        "{events:?}"
    );
    assert!(
        switch + 1 == applied,
        "ModelSwitch precedes its pick: {events:?}"
    );
    assert_eq!(
        session.model_override(ConfigMode::Act),
        Some("provider/after")
    );

    let assistant_models: Vec<Option<String>> = session
        .messages
        .iter()
        .filter(|message| message.role == opencoder_core::Role::Assistant)
        .map(|message| message.model.clone())
        .collect();
    assert_eq!(
        assistant_models,
        vec![Some("before".to_string()), Some("after".to_string())],
        "each assistant message records the model that produced it"
    );
}

/// A pick that cannot be persisted changes nothing: the runtime stays on the
/// previous model, no `ModelSwitch` is emitted, and the failure travels on the
/// pick notification. Once storage accepts writes the same pick applies.
#[tokio::test]
async fn store_failure_reports_the_pick_and_keeps_the_runtime_model() {
    let store = Arc::new(TursoStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: "mid-turn-model-pick-failure".into(),
            agent: Some("act".into()),
            model: Some("provider/before".into()),
            created_at: 1,
            updated_at: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    let conn = store.conn().await.unwrap();
    conn.execute(
        "CREATE TRIGGER reject_mid_turn_override BEFORE UPDATE OF model_overrides_json ON sessions BEGIN SELECT RAISE(FAIL, 'injected mid-turn override failure'); END",
        (),
    )
    .await
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "mid-turn-model-pick-failure",
        resolve_agent("act").unwrap(),
        config(),
        Arc::new(MockChatClient::new().with_default(vec![done()])),
        dir.path().to_path_buf(),
    )
    .with_store(store.clone());
    let queue = session.model_switch_queue.clone();

    queue_model_switch(&queue, ConfigMode::Act, "provider/rejected");
    let mut events = Vec::new();
    run(&mut session, "one".into(), |event| events.push(event))
        .await
        .unwrap();
    let applied = events
        .iter()
        .find(|event| matches!(event, SessionEvent::ModelOverrideApplied { .. }))
        .expect("the refused pick is reported");
    assert!(matches!(
        applied,
        SessionEvent::ModelOverrideApplied { mode, model, error: Some(error) }
            if mode == "act" && model == "provider/rejected" && error.contains("injected mid-turn override failure")
    ));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SessionEvent::ModelSwitch(_))),
        "a refused pick emits no ModelSwitch: {events:?}"
    );
    assert_eq!(session.effective_model(), "provider/before");
    assert_eq!(session.model_override(ConfigMode::Act), None);
    assert_eq!(
        pending_model_switches(&queue),
        0,
        "a failed pick is not retried"
    );

    conn.execute("DROP TRIGGER reject_mid_turn_override", ())
        .await
        .unwrap();
    queue_model_switch(&queue, ConfigMode::Act, "provider/after");
    events.clear();
    run(&mut session, "two".into(), |event| events.push(event))
        .await
        .unwrap();
    let switch = position(&events, |e| matches!(e, SessionEvent::ModelSwitch(_)));
    let applied = position(&events, |e| {
        matches!(e, SessionEvent::ModelOverrideApplied { error: None, .. })
    });
    assert!(switch + 1 == applied, "{events:?}");
    assert!(matches!(
        &events[switch],
        SessionEvent::ModelSwitch(model) if model == "after"
    ));
    assert_eq!(session.effective_model(), "provider/after");
    assert_eq!(
        store
            .get_session_model_overrides("mid-turn-model-pick-failure")
            .await
            .unwrap()
            .unwrap()
            .act
            .as_deref(),
        Some("provider/after")
    );
}

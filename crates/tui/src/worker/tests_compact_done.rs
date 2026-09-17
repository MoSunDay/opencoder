//! `/compact` Done-contract tests: the TUI worker's Compact arm must emit a
//! terminal `SessionEvent::Done` on success (persisted + forwarded), exactly
//! like the web `DrainCmd::Compact` arm. Without Done the app_loop never
//! re-syncs pending Queue/Steer rows, so inputs admitted during the
//! compaction turn strand in the store forever.

use super::*;

/// Fresh in-memory store with the session row created (session_events has a
/// FK to sessions).
async fn memory_store(sid: &str) -> Arc<dyn opencoder_store::Store> {
    let store: Arc<dyn opencoder_store::Store> =
        Arc::new(opencoder_store::LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&opencoder_store::SessionMeta {
            id: sid.into(),
            ..Default::default()
        })
        .await
        .unwrap();
    store
}

async fn admit_queued(store: &Arc<dyn opencoder_store::Store>, sid: &str, key: &str) {
    store
        .admit_input(&opencoder_store::SessionInput {
            seq: None,
            id: key.into(),
            session_id: sid.into(),
            delivery: opencoder_store::Delivery::Queue,
            prompt: "stranded prompt".into(),
            images: vec![],
            admitted_seq: 0,
            promoted_seq: None,
            display_text: None,
        })
        .await
        .unwrap();
}

/// A successful compaction (summary path) must end with Done — forwarded to
/// the UI bridge AND persisted — while a queued input admitted during the
/// compaction turn stays pending (Done is what re-arms the drain; the runner
/// consumes the row on the next drain turn, not here).
#[tokio::test]
async fn compact_with_summary_emits_done() {
    let sid = "compact-done";
    let store = memory_store(sid).await;
    admit_queued(&store, sid, "q-1").await;

    let mock: Arc<dyn opencoder_llm::ChatStream> = Arc::new(
        opencoder_llm::MockChatClient::new().with_default(vec![opencoder_llm::LlmEvent::Completed {
            text: "summary of old turns".into(),
            tool_calls: Vec::<opencoder_llm::CompletedToolCall>::new(),
            usage: None,
        }]),
    );
    let mut sess = SessionState::new(
        sid,
        opencoder_core::resolve_agent("act").unwrap(),
        opencoder_core::Config::default(),
        mock,
        std::env::temp_dir(),
    );
    sess.store = Some(store.clone());
    // Two turns so compaction_split finds a real head/tail split.
    sess.messages.push(opencoder_core::Message::user("u1", "first turn"));
    sess.messages.push(opencoder_core::Message::assistant("a1"));
    sess.messages.push(opencoder_core::Message::user("u2", "second turn"));
    sess.messages.push(opencoder_core::Message::assistant("a2"));

    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);
    let quit = process_cmd(UiCmd::Compact, &mut sess, &evt_tx).await;
    assert!(!quit);

    let events: Vec<SessionEvent> = std::iter::from_fn(|| evt_rx.try_recv().ok())
        .filter_map(|e| match e {
            UiEvent::Session(sev) => Some(sev),
            _ => None,
        })
        .collect();
    assert!(
        events.iter().any(|e| matches!(e, SessionEvent::Done)),
        "compact success must forward SessionEvent::Done, got {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::Compaction(_))),
        "the summary path still reports Compaction"
    );

    let kinds: Vec<String> = store
        .events_after(sid, 0)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|r| r.sse_kind)
        .collect();
    assert!(
        kinds.iter().any(|k| k == "done"),
        "Done must be persisted for SSE replay, got {kinds:?}"
    );
    assert!(
        !store
            .pending_inputs(sid, opencoder_store::Delivery::Queue)
            .await
            .unwrap()
            .is_empty(),
        "compact itself must not consume pending inputs; the drain turn does"
    );
}

/// The `Ok(None)` arm ("nothing to compact yet") is still a successful
/// command: web emits Done for every `Ok(_)`, so an empty transcript must
/// also produce the terminal Done frame.
#[tokio::test]
async fn compact_noop_still_emits_done() {
    let sid = "compact-noop-done";
    let store = memory_store(sid).await;
    admit_queued(&store, sid, "q-1").await;

    let mut sess = SessionState::new(
        sid,
        opencoder_core::resolve_agent("act").unwrap(),
        opencoder_core::Config::default(),
        Arc::new(opencoder_llm::MockChatClient::new()),
        std::env::temp_dir(),
    );
    sess.store = Some(store.clone());

    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);
    let _ = process_cmd(UiCmd::Compact, &mut sess, &evt_tx).await;

    let events: Vec<SessionEvent> = std::iter::from_fn(|| evt_rx.try_recv().ok())
        .filter_map(|e| match e {
            UiEvent::Session(sev) => Some(sev),
            _ => None,
        })
        .collect();
    assert!(
        events.iter().any(|e| matches!(e, SessionEvent::Done)),
        "Ok(None) compact must still forward Done, got {events:?}"
    );
    let kinds: Vec<String> = store
        .events_after(sid, 0)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|r| r.sse_kind)
        .collect();
    assert!(
        kinds.iter().any(|k| k == "done"),
        "Ok(None) Done must be persisted, got {kinds:?}"
    );
}

/// Failure keeps the web semantics: Error only, NO Done (no auto-restart /
/// error loop). Exhaust the mock so the summarizer LLM call fails.
#[tokio::test]
async fn compact_failure_emits_error_without_done() {
    let sid = "compact-fail";
    let store = memory_store(sid).await;
    let mut sess = SessionState::new(
        sid,
        opencoder_core::resolve_agent("act").unwrap(),
        opencoder_core::Config::default(),
        Arc::new(opencoder_llm::MockChatClient::new()), // exhausted: errors
        std::env::temp_dir(),
    );
    sess.store = Some(store.clone());
    sess.messages.push(opencoder_core::Message::user("u1", "first turn"));
    sess.messages.push(opencoder_core::Message::assistant("a1"));
    sess.messages.push(opencoder_core::Message::user("u2", "second turn"));

    let (evt_tx, mut evt_rx) = mpsc::channel::<UiEvent>(64);
    let _ = process_cmd(UiCmd::Compact, &mut sess, &evt_tx).await;

    let events: Vec<SessionEvent> = std::iter::from_fn(|| evt_rx.try_recv().ok())
        .filter_map(|e| match e {
            UiEvent::Session(sev) => Some(sev),
            _ => None,
        })
        .collect();
    assert!(
        events.iter().any(|e| matches!(e, SessionEvent::Error(_))),
        "failure must surface Error, got {events:?}"
    );
    assert!(
        !events.iter().any(|e| matches!(e, SessionEvent::Done)),
        "failure must NOT emit Done (no auto-restart error loops)"
    );
}

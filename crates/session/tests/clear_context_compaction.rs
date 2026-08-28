//! Regression: `/clear_context` with a preserved seed must not trip
//! compaction into "found nothing to summarize" and kill the run.
//!
//! Bug: `after_handoff` left `last_usage` stale — the model-reported usage
//! of the pre-clear conversation. The handoff transcript is exactly
//! one synthetic message, so on the next turn `should_compact` fired on the
//! stale reported usage (`>= budget`), `compaction_split` returned `None`
//! (single-message transcript has nothing to summarize), and `run_loop`
//! surfaced "compaction failed: transcript exceeds context window but
//! compaction found nothing to summarize" — killing the session even though
//! the fresh transcript was tiny.
//!
//! Fix: `after_handoff` / `after_compaction` reset `last_usage` (the reported
//! usage of a collapsed transcript is meaningless), and the runner's
//! `Ok(None)` branch proceeds when the current transcript still fits under
//! the hard context limit.

use std::sync::{Arc, Mutex};

use opencoder_core::{resolve_agent, Config, ContentBlock, Message};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{compaction::should_compact, run, SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, Store};

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn done_turn(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: Some(Usage::default()),
    }
}

#[tokio::test]
async fn clear_context_with_stale_usage_still_continues_seed() {
    let store = mem_store().await;
    store
        .create_session(&opencoder_store::SessionMeta {
            id: "cc-stale-usage".into(),
            agent: Some("sandbox".into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();

    let msgs = vec![Message::user("u1", "implement feature X"), {
        let mut m = Message::assistant("a1");
        m.blocks
            .push(ContentBlock::text("I will implement X by..."));
        m
    }];
    store
        .append_messages("cc-stale-usage", &msgs)
        .await
        .unwrap();

    let mock: Arc<MockChatClient> =
        Arc::new(MockChatClient::new().push_script(vec![done_turn("done")]));
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "cc-stale-usage",
        resolve_agent("sandbox").unwrap(),
        config(),
        mock.clone() as Arc<dyn ChatStream>,
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();
    session.messages = msgs.clone();
    // Stale model-reported usage from the pre-clear conversation:
    // with the old code this re-triggered should_compact against the
    // single-message handoff transcript and killed the run.
    session.last_usage = Usage {
        input_tokens: 500_000,
        ..Usage::default()
    };
    assert!(
        should_compact(&session),
        "precondition: stale reported usage must trip should_compact"
    );

    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let ev_collector = events.clone();
    let outcome = run(&mut session, "/clear_context".into(), move |ev| {
        if let Ok(mut g) = ev_collector.lock() {
            g.push(ev);
        }
    })
    .await;

    // 1) The run succeeds — no spurious compaction error after the collapse.
    assert!(
        outcome.is_ok(),
        "clear-context handoff must not be killed by stale-usage compaction, got {outcome:?}"
    );

    // 2) The seed turn reached the LLM exactly once.
    let requests = mock.requests();
    assert_eq!(
        requests.len(),
        1,
        "one LLM call to continue from the preserved seed"
    );

    // 3) The preserved reply appears in the model context (seed intact).
    let body = requests[0].to_body().to_string();
    assert!(
        body.contains("I will implement X by..."),
        "preserved reply must appear in the model context: {body}"
    );

    // 4) No compaction / error events were emitted.
    let collected = events.lock().unwrap().clone();
    assert!(
        !collected
            .iter()
            .any(|ev| matches!(ev, SessionEvent::Error(_) | SessionEvent::Compaction(_))),
        "no compaction or error events expected, got: {collected:?}"
    );
}

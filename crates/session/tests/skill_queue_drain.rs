//! Integration tests: drain priority between a sticky skill and pending
//! queue/steer inputs.
//!
//! Before the fix, `run_with_registry` computed `drain_mode = … && !has_skill`,
//! so an empty-prompt drain restart (TUI `drain_pending` / web
//! `drain_to_completion`) with a sticky skill FIRST injected a synthetic
//! `SKILL_TRIGGER` and ran another skill turn; the queue was only popped at a
//! later text-only idle boundary. Probabilistic interruption (cancel / LLM
//! Err / doom guard) skipped that boundary, stranding the row pending — and
//! the frontend resync restarted the drain, re-injecting the trigger again:
//! a self-continuing loop that repeatedly re-activated the same skill and
//! never popped the queue.
//!
//! Fix semantics: pending steers/queues win (FIFO). With nothing pending, an
//! empty submit still means "continue the active skill" (trigger injected).

mod common;

use std::sync::{Arc, Mutex};

use common::FlakyClaimStore;
use opencoder_core::{resolve_agent, Config, Role};
use opencoder_llm::{ChatStream, LlmEvent, MockChatClient, Usage};
use opencoder_session::{run, skill_resolve::SKILL_TRIGGER, SessionEvent, SessionState};
use opencoder_store::{Delivery, LibsqlStore, SessionInput, SessionMeta, Store};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

/// A mock script that streams one text delta then completes — produces a
/// `TextDelta` event the runner forwards to `on_event`.
fn stream_turn(text: &str) -> Vec<LlmEvent> {
    vec![
        LlmEvent::TextDelta(text.into()),
        LlmEvent::Completed {
            text: text.into(),
            tool_calls: vec![],
            usage: Some(Usage {
                input_tokens: 5,
                output_tokens: 3,
                total_tokens: 8,
                ..Default::default()
            }),
        },
    ]
}

async fn seed(store: &Arc<dyn Store>, id: &str) {
    store
        .create_session(&SessionMeta {
            id: id.into(),
            agent: Some("act".into()),
            model: Some("m/g".into()),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
}

fn mk_input(sid: &str, prompt: &str) -> SessionInput {
    SessionInput {
        seq: None,
        id: opencoder_session::runner::new_id(),
        session_id: sid.into(),
        delivery: Delivery::Queue,
        prompt: prompt.into(),
        images: vec![],
        display_text: None,
        admitted_seq: 0,
        promoted_seq: None,
    }
}

fn mk_session(id: &str, client: Arc<dyn ChatStream>, store: Arc<dyn Store>) -> SessionState {
    let dir = tempfile::tempdir().unwrap();
    SessionState::new(
        id,
        resolve_agent("act").unwrap(),
        config(),
        client,
        dir.path().to_path_buf(),
    )
    .with_store(store)
    .mark_session_created()
}

/// Collect events from one `run` call.
async fn run_collect(session: &mut SessionState, prompt: &str) -> Vec<SessionEvent> {
    let events: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    run(session, prompt.into(), move |ev| {
        sink.lock().unwrap().push(ev)
    })
    .await
    .unwrap();
    let out = events.lock().unwrap().clone();
    out
}

fn queue_consumed_count(evs: &[SessionEvent]) -> usize {
    evs.iter()
        .filter(|e| matches!(e, SessionEvent::QueueConsumed { .. }))
        .count()
}

fn trigger_count(session: &SessionState) -> usize {
    session
        .messages
        .iter()
        .filter(|m| m.role == Role::User && m.text() == SKILL_TRIGGER)
        .count()
}

// ---------------------------------------------------------------------------
// 1. sticky skill + pending queue + empty prompt -> queue first, no entry
//    SKILL_TRIGGER injection
// ---------------------------------------------------------------------------

/// The core regression: a drain restart with a sticky skill and a pending
/// queue item must pop the queue (QueueConsumed exactly once, one LLM turn
/// for the queued prompt) and must NOT inject an entry SKILL_TRIGGER that
/// would run another skill turn first.
#[tokio::test]
async fn sticky_skill_with_pending_queue_pops_queue_first() {
    let store = mem_store().await;
    seed(&store, "sq-1").await;

    let mock = Arc::new(MockChatClient::new().push_script(stream_turn("queued reply")));
    let client: Arc<dyn ChatStream> = mock.clone();
    let mut session = mk_session("sq-1", client, store.clone());
    session.set_skill(Some("STICKY SKILL BODY".into()));

    store
        .admit_input(&mk_input("sq-1", "queued task"))
        .await
        .unwrap();

    let evs = run_collect(&mut session, "").await;

    assert_eq!(queue_consumed_count(&evs), 1, "queue popped exactly once");
    assert_eq!(
        trigger_count(&session),
        0,
        "no entry SKILL_TRIGGER injected"
    );
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::TextDelta(t) if t.contains("queued reply"))),
        "queued prompt ran an LLM turn"
    );
    assert!(
        evs.iter().any(|e| matches!(e, SessionEvent::Done)),
        "run finished with Done"
    );
    let queued_text_count = session
        .messages
        .iter()
        .filter(|m| m.role == Role::User && m.text().contains("queued task"))
        .count();
    assert_eq!(queued_text_count, 1, "queued prompt recorded exactly once");
}

// ---------------------------------------------------------------------------
// 2. cancel interrupt + restart -> exactly one pop, no duplicate consumption
// ---------------------------------------------------------------------------

/// A hard cancel fires before the drain starts (the interrupted-run state);
/// the restart with an empty prompt must pop the queue exactly once. The
/// queue row must be promoted (not re-readable) after the restart.
#[tokio::test]
async fn cancel_then_restart_pops_queue_exactly_once() {
    let store = mem_store().await;
    seed(&store, "sq-2").await;

    let mock = Arc::new(
        MockChatClient::new()
            .push_script(stream_turn("queued reply"))
            // Phase 3's drain (sticky skill, nothing pending) legitimately
            // runs one more skill turn via the entry trigger.
            .with_default(stream_turn("skill reply")),
    );
    let client: Arc<dyn ChatStream> = mock.clone();
    let stale = CancellationToken::new();
    stale.cancel();
    let mut session = mk_session("sq-2", client, store.clone());
    session.set_skill(Some("STICKY SKILL BODY".into()));
    session.cancel = Some(stale);

    store
        .admit_input(&mk_input("sq-2", "queued task"))
        .await
        .unwrap();

    // Phase 1: cancelled run breaks at the top of run_loop ("interrupted").
    let evs1 = run_collect(&mut session, "").await;
    assert!(
        evs1.iter()
            .any(|e| matches!(e, SessionEvent::Status(s) if s == "interrupted")),
        "cancelled run short-circuits"
    );
    assert_eq!(
        queue_consumed_count(&evs1),
        0,
        "nothing popped while cancelled"
    );

    // Phase 2: fresh token (TUI ResetCancel equivalent) + empty-prompt drain.
    session.cancel = Some(CancellationToken::new());
    let evs2 = run_collect(&mut session, "").await;

    assert_eq!(
        queue_consumed_count(&evs2),
        1,
        "restart popped exactly once"
    );
    assert!(
        evs2.iter().any(|e| matches!(e, SessionEvent::Done)),
        "restart finished with Done"
    );
    let queued_text_count = session
        .messages
        .iter()
        .filter(|m| m.role == Role::User && m.text().contains("queued task"))
        .count();
    assert_eq!(queued_text_count, 1, "queued prompt recorded exactly once");

    // The queue row is promoted: a third drain sees nothing pending.
    let evs3 = run_collect(&mut session, "").await;
    assert_eq!(queue_consumed_count(&evs3), 0, "no double consumption");
}

// ---------------------------------------------------------------------------
// 3. regression guard: no pending + sticky skill + empty prompt -> trigger
// ---------------------------------------------------------------------------

/// With NOTHING pending, an empty submit still means "continue the active
/// skill": the entry path injects SKILL_TRIGGER and runs one skill turn.
#[tokio::test]
async fn sticky_skill_empty_prompt_no_pending_still_triggers() {
    let store = mem_store().await;
    seed(&store, "sq-3").await;

    let mock = Arc::new(MockChatClient::new().push_script(stream_turn("skill reply")));
    let client: Arc<dyn ChatStream> = mock.clone();
    let mut session = mk_session("sq-3", client, store);
    session.set_skill(Some("SKILL BODY".into()));

    let evs = run_collect(&mut session, "").await;

    assert_eq!(trigger_count(&session), 1, "entry SKILL_TRIGGER injected");
    assert_eq!(queue_consumed_count(&evs), 0, "nothing to pop");
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::TextDelta(t) if t.contains("skill reply"))),
        "skill turn ran"
    );
    assert!(evs.iter().any(|e| matches!(e, SessionEvent::Done)));
}

// ---------------------------------------------------------------------------
// 4. transient claim failure -> one retry recovers the pop
// ---------------------------------------------------------------------------

#[tokio::test]
async fn transient_claim_err_retries_once_and_pops() {
    let inner = mem_store().await;
    seed(&inner, "sq-4").await;
    let store: Arc<dyn Store> = Arc::new(FlakyClaimStore {
        inner,
        first_claim_failed: Mutex::new(false),
    });

    let mock = Arc::new(MockChatClient::new().push_script(stream_turn("queued reply")));
    let client: Arc<dyn ChatStream> = mock.clone();
    let mut session = mk_session("sq-4", client, store.clone());
    session.set_skill(Some("STICKY SKILL BODY".into()));

    store
        .admit_input(&mk_input("sq-4", "queued task"))
        .await
        .unwrap();

    let evs = run_collect(&mut session, "").await;

    assert_eq!(
        queue_consumed_count(&evs),
        1,
        "retry recovered the transient claim failure: popped exactly once"
    );
    assert!(
        evs.iter().any(|e| matches!(e, SessionEvent::Done)),
        "run completed instead of stranding the row"
    );
}

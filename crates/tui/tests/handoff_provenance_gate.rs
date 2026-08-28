//! The `/clear_context` gate is PURE TRANSCRIPT PROVENANCE: the newest
//! non-empty assistant reply is the thing preserved. There is no arming
//! counter, no turn-distance rule and no mode state — the same input always
//! produces the same fold, wherever the say sits in the transcript.

use std::sync::Arc;
use std::time::Duration;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message};
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_session::{is_clear_context_handoff, is_clear_context_seed, SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use opencoder_tui::worker::{process_cmd, UiCmd, UiEvent};
use tokio::sync::mpsc;

fn text_done(text: &str) -> LlmEvent {
    LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }
}

fn assistant_with_text(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m
}

async fn session_with_transcript(id: &str, msgs: Vec<Message>) -> (SessionState, Arc<MockChatClient>, Arc<dyn Store>) {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store
        .create_session(&SessionMeta {
            id: id.into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    for m in &msgs {
        store.append_message(id, m).await.unwrap();
    }
    // Any seeded fold falls through to one execution turn; give it a
    // deterministic completion so the turn lands a reply.
    let mock = Arc::new(
        MockChatClient::new().with_default(vec![text_done("ack from the model")]),
    );
    let mut sess = SessionState::new(
        id,
        resolve_agent("act").unwrap(),
        Config {
            model: "m/g".into(),
            ..Config::default()
        },
        mock.clone() as Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    )
    .with_store(store.clone())
    .mark_session_created();
    // Live transcript mirror (the store rows above are the durable half).
    sess.messages = msgs;
    (sess, mock, store)
}

async fn clear(sess: &mut SessionState) -> Vec<UiEvent> {
    let (tx, mut rx) = mpsc::channel::<UiEvent>(64);
    let quit = process_cmd(UiCmd::Prompt("/clear_context".into(), vec![]), sess, &tx).await;
    assert!(!quit);
    let mut events = Vec::new();
    let collect = async {
        while let Some(ev) = rx.recv().await {
            events.push(ev);
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(10), collect).await;
    events
}

fn reset_marker(events: &[UiEvent]) -> String {
    let msgs = events
        .iter()
        .find_map(|e| match e {
            UiEvent::Session(SessionEvent::TranscriptReset(m)) => Some(m.clone()),
            _ => None,
        })
        .expect("TranscriptReset emitted");
    assert_eq!(msgs.len(), 1, "the fold always leaves one marker message");
    msgs[0].text()
}

/// The say is preserved even when the user spoke AFTER it: provenance looks
/// at the newest assistant reply anywhere in the transcript — no arming
/// counter, no "must be the last message" rule (the legacy plan-handoff gate
/// had one; this contract must not).
#[tokio::test]
async fn newest_assistant_text_is_preserved_even_with_trailing_user_turns() {
    let (mut sess, mock, store) = session_with_transcript(
        "provenance-trailing",
        vec![
            Message::user("u1", "plan the migration"),
            assistant_with_text("a1", "the migration plan is final"),
            Message::user("u2", "ok wait, hold on"),
            Message::user("u3", "actually nevermind that"),
        ],
    )
    .await;

    let events = clear(&mut sess).await;
    let marker = reset_marker(&events);
    assert!(
        marker.contains("continuity context") && marker.contains("the migration plan is final"),
        "the say must travel as continuity context regardless of trailing user turns, got: {marker}"
    );
    assert_eq!(
        mock.call_count(),
        1,
        "a preserved say executes exactly one seeded turn"
    );
    let meta = store.get_session("provenance-trailing").await.unwrap().unwrap();
    assert!(is_clear_context_seed(meta.handoff_plan.as_deref().unwrap_or("")));

    // The seeded execution turn's reply is the only new content.
    assert_eq!(sess.messages.len(), 2, "seed + execution reply");
    assert!(
        sess.messages[1].text().contains("ack from the model"),
        "the execution reply lands after the seed, got: {:?}",
        sess.messages[1].text()
    );
}

/// An assistant message with only EMPTY text blocks carries no provenance:
/// the fold degrades to the blank fresh-start sentinel and no LLM turn runs.
#[tokio::test]
async fn empty_assistant_text_degrades_to_blank_sentinel() {
    let (mut sess, mock, store) = session_with_transcript(
        "provenance-empty",
        vec![
            Message::user("u1", "do something"),
            assistant_with_text("a1", ""), // streamed but never said anything
            Message::user("u2", "clear this"),
        ],
    )
    .await;

    let events = clear(&mut sess).await;
    let marker = reset_marker(&events);
    assert!(
        marker.contains("starting fresh"),
        "no preserved text -> blank fresh-start marker, got: {marker}"
    );
    assert!(
        !marker.contains("prior context, not a new instruction"),
        "the seed wrapper must not appear without a preserved say, got: {marker}"
    );
    assert_eq!(
        mock.call_count(),
        0,
        "the sentinel path stops without an LLM turn"
    );
    let meta = store.get_session("provenance-empty").await.unwrap().unwrap();
    assert!(is_clear_context_handoff(meta.handoff_plan.as_deref().unwrap_or("")));
    assert!(!is_clear_context_seed(meta.handoff_plan.as_deref().unwrap_or("")));
}

/// Repeated clears are deterministic and monotone: clear -> seed; clear again
/// (the seed is a USER message, so no assistant provenance remains) -> the
/// blank sentinel. The same input never yields different folds.
#[tokio::test]
async fn repeated_clears_are_deterministic_seed_then_sentinel() {
    let (mut sess, mock, store) = session_with_transcript(
        "provenance-repeat",
        vec![
            Message::user("u1", "build the thing"),
            assistant_with_text("a1", "the thing is built"),
        ],
    )
    .await;

    // First clear: the say is preserved and executed.
    let first = clear(&mut sess).await;
    let seed_marker = reset_marker(&first);
    assert!(seed_marker.contains("the thing is built"));
    assert!(is_clear_context_seed(
        store
            .get_session("provenance-repeat")
            .await
            .unwrap()
            .unwrap()
            .handoff_plan
            .as_deref()
            .unwrap_or("")
    ));
    assert_eq!(mock.call_count(), 1, "the seeded fold runs one turn");

    // Second clear: the transcript is now [seed, reply]. The reply is the
    // newest assistant text -> it becomes the new seed (deterministic rule,
    // not a special case).
    mock.queue_script(vec![text_done("second reply")]);
    let second = clear(&mut sess).await;
    let second_marker = reset_marker(&second);
    assert!(
        second_marker.contains("continuity context")
            && second_marker.contains("ack from the model"),
        "the newest say (the execution reply) is re-preserved on every clear, got: {second_marker}"
    );
    assert_eq!(mock.call_count(), 2, "still exactly one turn per seeded fold");
}

/// A synthetic marker can never serve as provenance for the NEXT fold: after
/// the blank sentinel fold, clearing again stays blank (the sentinel is a
/// user-role message with no assistant text to preserve).
#[tokio::test]
async fn blank_sentinel_never_arms_a_later_seed() {
    let (mut sess, mock, _store) = session_with_transcript(
        "provenance-noarm",
        vec![Message::user("u1", "a request that was never answered")],
    )
    .await;

    let first = clear(&mut sess).await;
    assert!(
        reset_marker(&first).contains("starting fresh"),
        "no assistant text -> blank sentinel"
    );
    assert_eq!(mock.call_count(), 0);

    // Clear again immediately: still blank, still no turn. No hidden state
    // (counter/flag) can flip the outcome.
    let second = clear(&mut sess).await;
    assert!(reset_marker(&second).contains("starting fresh"));
    assert_eq!(
        mock.call_count(),
        0,
        "repeated blank clears stay turn-free"
    );
}

//! Plan→act handoff: switching to act mode clears the transcript so the act
//! agent starts from only the final plan, not the full read-only planning
//! conversation.
//!
//! Contracts:
//! - With a finalized plan present, `handoff` collapses the transcript to a
//!   single synthetic user message whose text contains the plan and the
//!   execute-now directive.
//! - The newest non-empty assistant message wins as "the plan" (after any
//!   clarifying Q&A), and the store is left untouched.
//! - With no assistant plan text, `handoff` is a no-op (returns false, leaves
//!   messages unchanged) so callers fall back to current behavior.

use std::sync::Arc;

use opencode_core::{resolve_agent, ContentBlock, Config, Message, Role};
use opencode_llm::MockChatClient;
use opencode_session::{plan_handoff, SessionState};

fn config() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn assistant_with_text(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m
}

fn empty_session() -> SessionState {
    let agent = resolve_agent("act").unwrap();
    let mock = Arc::new(MockChatClient::new());
    let dir = tempfile::tempdir().unwrap();
    SessionState::new(
        "handoff-test",
        agent,
        config(),
        mock,
        dir.path().to_path_buf(),
    )
}

#[tokio::test]
async fn handoff_keeps_only_final_plan() {
    let mut session = empty_session();
    session.messages = vec![
        Message::user("u1", "build a foo"),
        assistant_with_text("a1", "exploring the codebase..."),
        Message::user("u2", "yes use option A"),
        // The finalized plan — newest assistant message, must win.
        assistant_with_text("a2", "## Plan\n1. do X\n2. do Y"),
    ];

    let reset = plan_handoff::handoff(&mut session);

    assert!(reset, "handoff should reset when a plan is present");
    assert_eq!(
        session.messages.len(),
        1,
        "transcript must collapse to a single handoff message"
    );
    let only = &session.messages[0];
    assert_eq!(only.role, Role::User, "handoff message is a user directive");
    assert!(only.synthetic, "handoff message is synthetic");
    let body = only.text();
    assert!(
        body.contains("## Plan\n1. do X\n2. do Y"),
        "handoff body must contain the final plan text, got: {body}"
    );
    assert!(
        body.to_lowercase().contains("execute"),
        "handoff body must instruct execution, got: {body}"
    );
    assert!(
        !body.contains("exploring the codebase"),
        "earlier planning chatter must be dropped, got: {body}"
    );
}

#[tokio::test]
async fn handoff_noop_without_plan() {
    let mut session = empty_session();
    session.messages = vec![Message::user("u1", "hello")];

    let reset = plan_handoff::handoff(&mut session);

    assert!(!reset, "handoff must be a no-op with no assistant plan");
    assert_eq!(session.messages.len(), 1, "messages must be unchanged");
    assert_eq!(session.messages[0].id, "u1");
}

#[tokio::test]
async fn handoff_skips_empty_assistant_turns() {
    let mut session = empty_session();
    session.messages = vec![
        Message::user("u1", "plan something"),
        // Empty assistant turn (e.g. a tool-only turn with no text) — skipped.
        Message::assistant("a1"),
        assistant_with_text("a2", "Final plan: ship it"),
    ];

    let reset = plan_handoff::handoff(&mut session);

    assert!(reset);
    let body = session.messages[0].text();
    assert!(
        body.contains("Final plan: ship it"),
        "must pick the non-empty assistant turn, got: {body}"
    );
}

#[tokio::test]
async fn handoff_does_not_touch_store() {
    use opencode_store::{LibsqlStore, Store};

    // Attach a real in-memory store and populate the durable transcript via
    // `record` (which persists). handoff must collapse the in-memory transcript
    // while leaving the durable store (the jsonl/audit surface) untouched.
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let mut session = empty_session();
    session.store = Some(store.clone());
    session.record(Message::user("u1", "build a thing")).await;
    session
        .record(assistant_with_text("a1", "## Plan\n1. step one\n2. step two"))
        .await;

    let before = store.load_messages(&session.id).await.unwrap();
    assert_eq!(before.len(), 2, "two messages persisted before handoff");

    let reset = plan_handoff::handoff(&mut session);

    assert!(reset, "handoff should reset when a plan is present");
    assert_eq!(
        session.messages.len(),
        1,
        "in-memory transcript must collapse to one handoff message"
    );
    let after = store.load_messages(&session.id).await.unwrap();
    assert_eq!(
        after.len(),
        2,
        "durable store must be unchanged after handoff (jsonl preserved)"
    );
    assert_eq!(after[0].id, "u1");
    assert!(after[1].text().contains("## Plan"), "plan text preserved in store");
}

#[test]
fn final_plan_text_picks_newest_nonempty_assistant() {
    let msgs = vec![
        assistant_with_text("a1", "early draft"),
        Message::user("u1", "tweak"),
        assistant_with_text("a2", "final plan"),
    ];
    let plan = plan_handoff::final_plan_text(&msgs);
    assert_eq!(plan.as_deref(), Some("final plan"));
}

#[test]
fn final_plan_text_none_when_no_assistant() {
    let msgs = vec![Message::user("u1", "hi")];
    assert_eq!(plan_handoff::final_plan_text(&msgs), None);
}

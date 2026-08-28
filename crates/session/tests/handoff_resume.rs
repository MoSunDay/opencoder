//! Transcript handoff boundary survives resume (Gap A).
//!
//! The store is append-only, so the full pre-handoff history stays durable.
//! But resume must reconstruct the FOCUSED post-handoff transcript — the
//! synthetic directive plus only the messages that followed — not replay the
//! exploration chatter. Mirrors compaction's trim+prepend pattern.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message, Role};
use opencoder_llm::MockChatClient;
use opencoder_session::{handoff, resume, SessionState};
use opencoder_store::{LibsqlStore, SessionPatch, Store};

fn cfg() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn assistant(id: &str, text: &str) -> Message {
    let mut m = Message::assistant(id);
    m.blocks.push(ContentBlock::text(text));
    m
}

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

#[tokio::test]
async fn resume_after_handoff_reconstructs_focused_transcript() {
    let store = mem_store().await;
    store
        .create_session(&opencoder_store::SessionMeta {
            id: "s1".into(),
            title: None,
            agent: Some("act".into()),
            model: Some("m".into()),

            autopilot_mode: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: None,
            requirement: None,
        })
        .await
        .unwrap();

    // Persist the exploration transcript via the store (append-only).
    let head_msgs = vec![
        Message::user("u1", "build a foo"),
        assistant("a1", "exploring the codebase..."),
        Message::user("u2", "yes use option A"),
        assistant("a2", "## Brief\n1. do X\n2. do Y"),
    ];
    store.append_messages("s1", &head_msgs).await.unwrap();
    let n_head = head_msgs.len();

    // Mirror the in-memory state and perform the handoff.
    let agent = resolve_agent("act").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut session = SessionState::new(
        "s1",
        agent,
        cfg(),
        Arc::new(MockChatClient::new()),
        dir.path().to_path_buf(),
    )
    .with_store(store.clone())
    .mark_session_created();
    session.messages = head_msgs.clone();
    // The handoff extracts the newest non-synthetic assistant text as the
    // brief (the autopilot ACT phase calls exactly this).
    let display = handoff::reset_to_directive(&mut session, "").expect("brief present");
    assert_eq!(
        session.messages.len(),
        1,
        "handoff collapses in-memory transcript"
    );
    assert_eq!(
        session.handoff_seq,
        Some(n_head as i64),
        "handoff_seq == number of pre-handoff store messages"
    );
    assert_eq!(session.handoff_plan.as_deref(), Some(display.as_str()));

    // Persist the boundary (mirrors the TUI worker's update_session call).
    store
        .update_session(
            "s1",
            &SessionPatch {
                handoff_seq: session.handoff_seq,
                handoff_plan: session.handoff_plan.clone(),
                updated_at: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Simulate the act agent recording a message after the handoff.
    let act_msg = assistant("act1", "executing step 1");
    store.append_message("s1", &act_msg).await.unwrap();

    // Resume: must reconstruct [directive, act_msg], NOT the full
    // pre-handoff history.
    let resumed = resume(
        store,
        "s1",
        cfg(),
        Arc::new(MockChatClient::new()),
        dir.path().to_path_buf(),
    )
    .await
    .unwrap();

    assert_eq!(
        resumed.messages.len(),
        2,
        "resumed transcript must be directive + act msg only"
    );
    let directive = &resumed.messages[0];
    assert_eq!(directive.role, Role::User);
    assert!(directive.synthetic, "handoff directive is synthetic");
    let body = directive.text();
    assert!(
        body.contains("## Brief\n1. do X\n2. do Y"),
        "brief text must be present, got: {body}"
    );
    assert!(
        body.to_lowercase().contains("execute"),
        "directive prefix must be present, got: {body}"
    );
    assert!(
        !body.contains("exploring the codebase"),
        "exploration chatter must be dropped, got: {body}"
    );
    assert_eq!(resumed.messages[1].id, "act1");
    assert_eq!(resumed.handoff_seq, Some(n_head as i64));
}

//! Plan→act handoff boundary survives resume (Gap A).
//!
//! The store is append-only, so the full plan-mode history stays durably. But
//! resume must reconstruct the FOCUSED post-handoff transcript — the synthetic
//! plan instruction plus only the act-mode messages that followed — not replay
//! the planning chatter. Mirrors compaction's trim+prepend pattern.

use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message, Role};
use opencoder_llm::MockChatClient;
use opencoder_session::{plan_handoff, resume, SessionState};
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
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
        })
        .await
        .unwrap();

    // Persist the plan-mode transcript via the store (append-only).
    let plan_msgs = vec![
        Message::user("u1", "build a foo"),
        assistant("a1", "exploring the codebase..."),
        Message::user("u2", "yes use option A"),
        assistant("a2", "## Plan\n1. do X\n2. do Y"),
    ];
    store.append_messages("s1", &plan_msgs).await.unwrap();
    let n_plan = plan_msgs.len();

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
    session.messages = plan_msgs.clone();

    let display = plan_handoff::handoff(&mut session, "").expect("plan present");
    assert_eq!(session.messages.len(), 1, "handoff collapses in-memory transcript");
    assert_eq!(
        session.handoff_seq,
        Some(n_plan as i64),
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

    // Resume: must reconstruct [plan_instruction, act_msg], NOT the full
    // plan-mode history.
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
        "resumed transcript must be plan instruction + act msg only"
    );
    let plan_msg = &resumed.messages[0];
    assert_eq!(plan_msg.role, Role::User);
    assert!(plan_msg.synthetic, "handoff instruction is synthetic");
    let body = plan_msg.text();
    assert!(
        body.contains("## Plan\n1. do X\n2. do Y"),
        "plan text must be present, got: {body}"
    );
    assert!(
        body.to_lowercase().contains("execute"),
        "directive prefix must be present, got: {body}"
    );
    assert!(
        !body.contains("exploring the codebase"),
        "planning chatter must be dropped, got: {body}"
    );
    assert_eq!(resumed.messages[1].id, "act1");
    assert_eq!(resumed.handoff_seq, Some(n_plan as i64));
}

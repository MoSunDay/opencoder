//! Verifies the resume compaction path loads ONLY the post-compaction tail via
//! `load_messages_after` (OFFSET) and rebuilds the synthetic summary from the
//! PERSISTED `summary_images`, never touching the soft-deleted compacted head.
//!
//! This is the fix for long-session resume stalls: previously resume reloaded
//! the entire (potentially huge) head just to re-derive a few image URLs.

use std::sync::Arc;

use opencoder_core::{Config, ContentBlock, Message, Role};
use opencoder_llm::MockChatClient;
use opencoder_session::resume;
use opencoder_store::{LibsqlStore, SessionMeta, SessionPatch, Store};

fn cfg() -> Config {
    Config {
        model: "m/g".into(),
        ..Config::default()
    }
}

fn user(id: &str, text: &str) -> Message {
    Message::user(id, text)
}

fn assistant(id: &str) -> Message {
    Message::assistant(id)
}

fn image_urls(msg: &Message) -> Vec<String> {
    msg.blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Image { url, .. } => Some(url.clone()),
            _ => None,
        })
        .collect()
}

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

fn meta(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        agent: Some("act".into()),
        model: Some("m".into()),
        created_at: 0,
        updated_at: 0,
        ..Default::default()
    }
}

#[tokio::test]
async fn resume_compaction_loads_only_tail_and_persisted_images() {
    let store = mem_store().await;
    store.create_session(&meta("inc1")).await.unwrap();

    // 8 messages: head (first 5) is "compacted", tail is the last 3.
    let all = vec![
        user("u0", "head 0"),
        assistant("a0"),
        user("u1", "head 1"),
        assistant("a1"),
        user("u2", "head 2"), // <- index 4; head = messages[..5]
        user("t0", "tail 0"), // <- index 5; tail = messages[5..]
        assistant("t1"),
        user("t2", "tail 2"),
    ];
    let n = all.len();
    let skip = 5; // summary_seq
    store.append_messages("inc1", &all).await.unwrap();

    // Persist a compaction boundary with a surviving image URL.
    store
        .update_session(
            "inc1",
            &SessionPatch {
                summary_seq: Some(skip as i64),
                summary: Some("[Conversation summary so far] compacted".into()),
                summary_images: Some(vec!["survived.png".into()]),
                updated_at: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let resumed = resume(
        store,
        "inc1",
        cfg(),
        Arc::new(MockChatClient::new()),
        std::env::temp_dir(),
    )
    .await
    .expect("resume must succeed");

    // messages = [summary] + tail(3) = 4. The 5 head messages are NOT loaded.
    assert_eq!(
        resumed.messages.len(),
        1 + (n - skip),
        "only the summary + tail are loaded; head is skipped"
    );

    // The summary message is synthetic and carries the PERSISTED image.
    let summary = &resumed.messages[0];
    assert_eq!(summary.role, Role::User);
    assert!(summary.synthetic);
    assert!(summary.text().starts_with("[Conversation summary so far]"));
    assert_eq!(
        image_urls(summary),
        vec!["survived.png".to_string()],
        "summary image comes from the persisted field, not the reloaded head"
    );

    // The tail is exactly messages[skip..] -- proving the head was never loaded.
    let tail_ids: Vec<&str> = resumed.messages[1..].iter().map(|m| m.id.as_str()).collect();
    assert_eq!(tail_ids, vec!["t0", "t1", "t2"]);

    // No head message id leaks into the resumed transcript.
    for m in &resumed.messages {
        assert!(
            !m.id.starts_with('u') || m.synthetic,
            "head user message {} must not be reloaded",
            m.id
        );
        assert!(!m.id.starts_with('a'), "head assistant {} must not be reloaded", m.id);
    }
    // (u0/u1/u2 are head users; t2 is a tail user but non-synthetic -- the
    // guard above checks ids that start with 'u' OR 'a'; t2 starts with 't'.)
}

/// Without a persisted summary_seq, resume still does a full load (no-compaction
/// path) -- regression guard for the branching load strategy.
#[tokio::test]
async fn resume_without_compaction_loads_everything() {
    let store = mem_store().await;
    store.create_session(&meta("inc2")).await.unwrap();
    let all = vec![user("u0", "hi"), assistant("a0"), user("u1", "again")];
    store.append_messages("inc2", &all).await.unwrap();

    let resumed = resume(
        store,
        "inc2",
        cfg(),
        Arc::new(MockChatClient::new()),
        std::env::temp_dir(),
    )
    .await
    .expect("resume must succeed");

    assert_eq!(resumed.messages.len(), 3, "no compaction -> full load");
    assert_eq!(resumed.messages[0].id, "u0");
}

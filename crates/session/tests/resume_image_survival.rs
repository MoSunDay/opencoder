//! RC4: images must survive session resume.
//!
//! Both resume branches (compaction summary and transcript handoff) re-derive the
//! recent head images via `collect_head_images` and attach them to the
//! reconstructed synthetic message, so a vision model still sees them after a
//! crash/restart. These tests assert that contract against a durable store.

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

fn user_with_image(id: &str, text: &str, url: &str) -> Message {
    let mut m = Message::user(id, text);
    m.blocks.push(ContentBlock::Image {
        url: url.into(),
        detail: None,
    });
    m
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
    }
}

/// Compaction branch: a prior compaction set summary_seq; the head carried an
/// image. Resume must re-attach that image to the synthetic summary message.
#[tokio::test]
async fn resume_after_compaction_preserves_head_image() {
    let store = mem_store().await;
    store.create_session(&meta("c1")).await.unwrap();

    // Head (to be summarized) carries an image; tail does not.
    let head = vec![
        user_with_image("u1", "look here", "img1.png"),
        assistant("a1"),
    ];
    store.append_messages("c1", &head).await.unwrap();
    let tail = vec![Message::user("u2", "follow up"), assistant("a2")];
    store.append_messages("c1", &tail).await.unwrap();

    // Persist the compaction boundary.
    store
        .update_session(
            "c1",
            &SessionPatch {
                summary_seq: Some(head.len() as i64),
                // Images must now be PERSISTED to survive resume: the compacted
                // head is no longer reloaded (it is skipped via OFFSET), so the
                // surviving image URLs are read back from this persisted field.
                summary_images: Some(vec!["img1.png".into()]),
                summary: Some("[Conversation summary so far] discussed an image".into()),
                updated_at: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let resumed = resume(
        store,
        "c1",
        cfg(),
        Arc::new(MockChatClient::new()),
        std::env::temp_dir(),
    )
    .await
    .expect("resume must succeed");

    // Reconstructed transcript = [summary(+image), u2, a2].
    assert_eq!(resumed.messages.len(), 3);
    let summary = &resumed.messages[0];
    assert_eq!(summary.role, Role::User);
    assert!(summary.text().starts_with("[Conversation summary so far]"));
    let urls = image_urls(summary);
    assert_eq!(urls, vec!["img1.png"], "head image must survive resume");
}

/// Handoff branch: a transcript handoff set handoff_seq; the pre-handoff head
/// carried an image. Resume must re-attach it to the handoff directive. The
/// persisted `handoff_plan` uses legacy wording on purpose: legacy boundary
/// rows must reconstruct with the directive wording too.
#[tokio::test]
async fn resume_after_handoff_preserves_head_image() {
    let store = mem_store().await;
    store.create_session(&meta("h1")).await.unwrap();

    let head = vec![
        user_with_image("u1", "explore from this", "plan_img.png"),
        assistant("a1"),
        Message::user("u2", "approve"),
        assistant("a2"),
    ];
    store.append_messages("h1", &head).await.unwrap();
    store
        .append_message("h1", &assistant("act1"))
        .await
        .unwrap();

    store
        .update_session(
            "h1",
            &SessionPatch {
                handoff_seq: Some(head.len() as i64),
                handoff_plan: Some("## Plan\n1. do it".into()),
                updated_at: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let resumed = resume(
        store,
        "h1",
        cfg(),
        Arc::new(MockChatClient::new()),
        std::env::temp_dir(),
    )
    .await
    .expect("resume must succeed");

    // Reconstructed transcript = [handoff(+image), act1].
    assert_eq!(resumed.messages.len(), 2);
    let handoff = &resumed.messages[0];
    assert_eq!(handoff.role, Role::User);
    assert!(handoff.synthetic, "handoff instruction is synthetic");
    assert!(handoff.text().contains("## Plan"));
    let urls = image_urls(handoff);
    assert_eq!(
        urls,
        vec!["plan_img.png"],
        "pre-handoff head image must survive"
    );
}

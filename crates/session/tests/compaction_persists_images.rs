//! Verifies compaction persists the head's surviving image URLs to the store's
//! `summary_images` column, so resume can rebuild the summary WITHOUT reloading
//! the compacted head.

use std::collections::HashMap;
use std::sync::Arc;

use opencoder_core::{resolve_agent, Config, ContentBlock, Message};
use opencoder_llm::{ChatStream, CompletedToolCall, LlmEvent, MockChatClient, Usage};
use opencoder_session::compaction::compact;
use opencoder_session::SessionState;
use opencoder_store::{LibsqlStore, SessionMeta, Store};

fn cfg() -> Config {
    Config {
        model: "main/glm-5.2".into(),
        ..Config::default()
    }
}

fn meta(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        agent: Some("act".into()),
        model: Some("main/glm-5.2".into()),
        created_at: 0,
        updated_at: 0,
        ..Default::default()
    }
}

#[tokio::test]
async fn compaction_persists_surviving_images_to_store() {
    let store: Arc<dyn Store> = Arc::new(LibsqlStore::open_memory().await.unwrap());
    store.create_session(&meta("cp1")).await.unwrap();

    let mock: Arc<dyn ChatStream> = Arc::new(MockChatClient::new().with_default(vec![
        LlmEvent::TextDelta("summary of talk".into()),
        LlmEvent::Completed {
            text: "summary of talk".into(),
            tool_calls: Vec::<CompletedToolCall>::new(),
            usage: Some(Usage {
                input_tokens: 1,
                output_tokens: 1,
                total_tokens: 2,
                ..Usage::default()
            }),
        },
    ]));
    let agent = resolve_agent("act").expect("act agent");
    let mut s = SessionState::new("cp1", agent, cfg(), mock, std::env::temp_dir())
        .with_store(store.clone());

    // Two turns; head (u1+a1) carries an image, tail (u2+a2) does not.
    let mut u1 = Message::user("u1", "look at this");
    u1.blocks.push(ContentBlock::Image {
        url: "data:image/png;base64,AAAA".into(),
        detail: None,
    });
    let a1 = Message::assistant("a1");
    let u2 = Message::user("u2", "second");
    let a2 = Message::assistant("a2");
    // Persist the full transcript to the store (as record()/persist() would in
    // the real flow) so resume can read the tail back via OFFSET after the head
    // is compacted. compact() works on in-memory messages but resume reads the
    // store -- both must agree.
    store
        .append_messages("cp1", &[u1.clone(), a1.clone(), u2.clone(), a2.clone()])
        .await
        .unwrap();
    s.messages.push(u1);
    s.messages.push(a1);
    s.messages.push(u2);
    s.messages.push(a2);

    let outcome = compact(&mut s, &HashMap::new(), &mut |_| {}).await;
    assert!(outcome.is_ok(), "compaction must succeed: {outcome:?}");

    // The in-memory summary carries the image (existing contract)...
    assert!(s.messages[0].has_image(), "summary message carries the preserved image");

    // ...AND the store row now persists the image URLs + compaction metadata,
    // so resume can rebuild the summary without reloading the head.
    let m = store.get_session("cp1").await.unwrap().unwrap();
    assert_eq!(
        m.summary_images,
        vec!["data:image/png;base64,AAAA".to_string()],
        "surviving images persisted to summary_images"
    );
    assert!(m.summary_seq.is_some(), "summary_seq persisted");
    assert!(m.summary.is_some(), "summary text persisted");
    // In-memory state mirrors the store.
    assert_eq!(s.summary_images, m.summary_images);

    // A subsequent resume must see exactly one summary message + the tail,
    // with the image sourced from the persisted field.
    let resumed = opencoder_session::resume(
        store,
        "cp1",
        cfg(),
        Arc::new(MockChatClient::new()),
        std::env::temp_dir(),
    )
    .await
    .expect("resume must succeed");
    // summary(1) + tail u2,a2 = 3. The compacted head (u1,a1) is NOT reloaded.
    assert_eq!(resumed.messages.len(), 3, "resume loads only summary + tail");
    let urls: Vec<String> = resumed.messages[0]
        .blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Image { url, .. } => Some(url.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        urls,
        vec!["data:image/png;base64,AAAA".to_string()],
        "resume rebuilds summary image from persisted field"
    );
}

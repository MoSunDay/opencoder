use opencoder_core::{fleet::MessageCursor, Message};
use opencoder_store::{LibsqlStore, SessionMeta, Store};

#[tokio::test]
async fn bounded_transcript_preserves_display_synthetic_usage_and_utf8() {
    let store = LibsqlStore::open_memory().await.unwrap();
    store
        .create_session(&SessionMeta {
            id: "transcript".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut message = Message::user("input", "model input");
    message.display = Some("原始 $skill @file ".repeat(40_000));
    message.synthetic = true;
    message.usage.total_tokens = 123;
    store.append_message("transcript", &message).await.unwrap();
    let mut cursor = MessageCursor::default();
    let mut bytes = Vec::new();
    loop {
        let page = store
            .load_transcript_page("transcript", cursor, 8192, 16384)
            .await
            .unwrap();
        assert!(page.chunks.iter().map(|c| c.bytes.len()).sum::<usize>() <= 16384);
        for chunk in page.chunks {
            assert!(chunk.bytes.len() <= 8192);
            bytes.extend(chunk.bytes);
        }
        let Some(next) = page.next_cursor else { break };
        assert_ne!(next, cursor);
        cursor = next;
    }
    let decoded: Message = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded.id, message.id);
    assert_eq!(decoded.text(), message.text());
    assert_eq!(decoded.display, message.display);
    assert!(decoded.synthetic);
    assert_eq!(decoded.usage.total_tokens, 123);
    assert!(decoded.provider_state.is_none());
}

//! Message-store contracts for the rdb backend, ported from the SQL suites:
//! contiguous batch seqs, offset semantics, bounded raw/transcript paging
//! and role round trips.

use super::*;

#[tokio::test]
async fn message_append_load_and_offset_semantics() {
    let store = mem().await;
    store.create_session(&meta_for("s1", 1)).await.unwrap();
    let msgs: Vec<Message> = (0..10)
        .map(|i| {
            if i % 2 == 0 {
                user_msg(&format!("u{i}"), &format!("msg {i}"))
            } else {
                let mut m = Message::assistant(format!("a{i}"));
                m.blocks = vec![ContentBlock::text(format!("msg {i}"))];
                m
            }
        })
        .collect();

    // Batch append assigns contiguous per-session seqs from 1.
    let seqs = store.append_messages("s1", &msgs).await.unwrap();
    assert_eq!(seqs, (1..=10).collect::<Vec<i64>>());
    // Single append continues the range.
    assert_eq!(
        store
            .append_message("s1", &user_msg("u10", "tail"))
            .await
            .unwrap(),
        11
    );

    let loaded = store.load_messages("s1").await.unwrap();
    assert_eq!(loaded.len(), 11);
    assert_eq!(loaded[0].id, "u0");
    assert_eq!(loaded[10].id, "u10");
    assert_eq!(store.last_message_seq("s1").await.unwrap(), 11);

    // OFFSET semantics: skip the head without changing tail order.
    let tail = store.load_messages_after("s1", 8).await.unwrap();
    assert_eq!(texts(&tail), vec!["msg 8", "msg 9", "tail"]);
    assert!(store
        .load_messages_after("s1", 99)
        .await
        .unwrap()
        .is_empty());
    // Negative skip clamps to zero.
    assert_eq!(store.load_messages_after("s1", -3).await.unwrap().len(), 11);
    // Empty session: no error, empty vec, seq 0.
    assert!(store.load_messages("nope").await.unwrap().is_empty());
    assert_eq!(store.last_message_seq("nope").await.unwrap(), 0);

    // Raw rows carry the true seqs the resume boundary needs.
    let rows = store.load_message_rows("s1").await.unwrap();
    assert_eq!(rows.len(), 11);
    assert_eq!(rows[0].seq, 1);
    assert_eq!(rows[10].seq, 11);
    assert_eq!(rows[0].role, "user");

    // Usage update targets exactly one row.
    let usage = MessageUsage {
        total_tokens: 4321,
        ..Default::default()
    };
    store.set_message_usage("s1", "a1", &usage).await.unwrap();
    assert_eq!(
        store.load_messages("s1").await.unwrap()[1]
            .usage
            .total_tokens,
        4321
    );
    // Missing or duplicated message id is an error, never a silent write.
    assert!(store
        .set_message_usage("s1", "ghost", &usage)
        .await
        .is_err());
}

#[tokio::test]
async fn bounded_message_pages_preserve_bytes_utf8_and_terminate() {
    let store = mem().await;
    store.create_session(&meta_for("s1", 1)).await.unwrap();
    let mut message = user_msg("m1", "model input");
    message.display = Some("原始 $skill @file ".repeat(40_000));
    message.synthetic = true;
    message.usage.total_tokens = 123;
    store.append_message("s1", &message).await.unwrap();
    let mut tail = Message::assistant("m2");
    tail.blocks = vec![ContentBlock::text("second message")];
    store.append_message("s1", &tail).await.unwrap();

    // Raw page: chunk-bytes and row budget both honored; loop terminates;
    // chunks are grouped per message seq (each group is one JSON value).
    let (mut cursor, mut raw_groups) = (
        MessageCursor::default(),
        std::collections::BTreeMap::<i64, Vec<u8>>::new(),
    );
    loop {
        let page = store
            .load_message_page("s1", cursor, 8192, 16384)
            .await
            .unwrap();
        assert!(page.chunks.iter().map(|c| c.bytes.len()).sum::<usize>() <= 16384);
        assert!(page.chunks.iter().all(|c| c.bytes.len() <= 8192));
        for chunk in page.chunks {
            raw_groups.entry(chunk.seq).or_default().extend(chunk.bytes);
        }
        let Some(next) = page.next_cursor else { break };
        assert_ne!(next, cursor, "cursor must advance");
        cursor = next;
    }
    assert_eq!(raw_groups.len(), 2, "both messages page out");
    // A raw group is the stored blocks value: it parses standalone (the
    // raw lane carries blocks only, not display metadata).
    for group in raw_groups.values() {
        let blocks: Vec<ContentBlock> = serde_json::from_slice(group).unwrap();
        assert_eq!(blocks.len(), 1);
    }

    // Transcript page: each group reassembles to a loss-free display
    // projection (no provider-private state).
    let (mut cursor, mut groups) = (
        MessageCursor::default(),
        std::collections::BTreeMap::<i64, Vec<u8>>::new(),
    );
    loop {
        let page = store
            .load_transcript_page("s1", cursor, 8192, 16384)
            .await
            .unwrap();
        assert!(page.chunks.iter().map(|c| c.bytes.len()).sum::<usize>() <= 16384);
        for chunk in page.chunks {
            groups.entry(chunk.seq).or_default().extend(chunk.bytes);
        }
        let Some(next) = page.next_cursor else { break };
        cursor = next;
    }
    assert!(
        groups[&1].len() > 64 * 1024,
        "the oversized display field spans several chunks"
    );
    let decoded: Message = serde_json::from_slice(&groups[&1]).unwrap();
    assert_eq!(decoded.id, message.id);
    assert_eq!(decoded.display, message.display);
    assert!(decoded.synthetic);
    assert_eq!(decoded.usage.total_tokens, 123);
    assert!(
        decoded.provider_state.is_none(),
        "no provider-private state in transcripts"
    );
    assert_eq!(decoded.text(), message.text());
}

#[tokio::test]
async fn role_round_trip_keeps_assistant_and_user_distinct() {
    let store = mem().await;
    store.create_session(&meta_for("s1", 1)).await.unwrap();
    let mut a = Message::assistant("a1");
    a.blocks = vec![ContentBlock::text("reply")];
    store
        .append_messages("s1", &[user_msg("u1", "ask"), a])
        .await
        .unwrap();
    let loaded = store.load_messages("s1").await.unwrap();
    assert_eq!(loaded[0].role, Role::User);
    assert_eq!(loaded[1].role, Role::Assistant);
    assert_eq!(texts(&loaded), vec!["ask", "reply"]);
}

#[tokio::test]
async fn append_into_missing_session_replicates_the_foreign_key_failure() {
    let store = mem().await;
    // No session row: SQL `messages.session_id REFERENCES sessions(id)`
    // rejects the insert; so must the rdb backend.
    let err = store
        .append_messages("missing", &[user_msg("u1", "orphan")])
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("FOREIGN KEY constraint failed"),
        "{err}"
    );
    // An empty batch never reaches the FK check in SQL; it stays a no-op.
    assert!(store
        .append_messages("missing", &[])
        .await
        .unwrap()
        .is_empty());
}

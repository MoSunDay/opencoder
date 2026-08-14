//! Bug #14: `append_messages` persists messages in batches of `BATCH_CHUNK`
//! (200) per transaction. This is **non-atomic**: if a later batch fails, the
//! earlier batches remain committed (see the doc comment on `append_many`).
//!
//! These tests verify the documented multi-batch behavior: a payload larger
//! than one chunk is split across multiple transactions yet still lands
//! entirely, with strictly increasing sequence numbers and preserved order.
//! `BATCH_CHUNK` is a store-internal constant (200); we append more than that
//! to cross the chunk boundary.

use opencoder_core::{ContentBlock, Message, Role};
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use tempfile::TempDir;

fn msg(id: &str) -> Message {
    Message {
        id: id.into(),
        role: Role::User,
        blocks: vec![ContentBlock::text(format!("body for {id}"))],
        model: None,
        agent: None,
        usage: Default::default(),
        created_at: 0,
        synthetic: false,
    }
}

async fn fresh() -> (TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    store
        .create_session(&SessionMeta {
            id: "s1".into(),
            created_at: 0,
            updated_at: 0,
            ..Default::default()
        })
        .await
        .unwrap();
    (dir, store)
}

#[tokio::test]
async fn multi_batch_append_persists_all_in_order() {
    // 250 messages span two chunks (200 + 50). All must persist despite the
    // batch boundary; non-atomicity only matters on failure, which we don't
    // trigger here.
    let (_dir, store) = fresh().await;
    let msgs: Vec<Message> = (0..250).map(|i| msg(&format!("m{i}"))).collect();
    let seqs = store.append_messages("s1", &msgs).await.unwrap();

    // One seq number per message, strictly increasing.
    assert_eq!(seqs.len(), 250, "one seq returned per appended message");
    let mut sorted = seqs.clone();
    sorted.sort_unstable();
    assert_eq!(seqs, sorted, "seqs must be returned in ascending order");
    assert_eq!(sorted.first().copied(), Some(1), "first seq starts at 1");

    // Read back: all 250 present, in insertion order.
    let loaded = store.load_messages("s1").await.unwrap();
    assert_eq!(loaded.len(), 250, "all messages persisted across chunk boundary");
    for (i, m) in loaded.iter().enumerate() {
        assert_eq!(m.id, format!("m{i}"), "order preserved at index {i}");
    }
}

#[tokio::test]
async fn single_message_append_returns_one_seq() {
    // The common path: append_messages with a single message returns exactly
    // one seq number (used by the single-append delegation path).
    let (_dir, store) = fresh().await;
    let seqs = store.append_messages("s1", &[msg("only")]).await.unwrap();
    assert_eq!(seqs.len(), 1, "single-message append returns one seq");
    let loaded = store.load_messages("s1").await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "only");
}

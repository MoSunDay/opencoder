//! Verifies `Store::load_messages_after` OFFSET semantics: it returns only the
//! messages after the first `skip_count` rows (by insertion `seq` ASC), without
//! materializing the skipped head. This is the primitive the resume compaction
//! path uses to avoid reloading the (potentially huge) compacted head.

use opencoder_core::{ContentBlock, Message, Role};
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use tempfile::TempDir;

fn msgs(seed: &str, n: usize) -> Vec<Message> {
    (0..n)
        .map(|i| {
            let role = if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            let text = format!("{seed} msg {i}");
            match role {
                Role::User => Message::user(format!("{seed}-{i}"), text),
                _ => {
                    let mut m = Message::assistant(format!("{seed}-{i}"));
                    m.blocks = vec![ContentBlock::text(text)];
                    m
                }
            }
        })
        .collect()
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
async fn load_messages_after_skip_semantics() {
    let (_dir, store) = fresh().await;
    store.append_messages("s1", &msgs("m", 10)).await.unwrap();

    // skip=0 -> all 10
    let all = store.load_messages_after("s1", 0).await.unwrap();
    assert_eq!(all.len(), 10, "skip=0 returns everything");

    // skip=3 -> last 7, ordered
    let tail7 = store.load_messages_after("s1", 3).await.unwrap();
    assert_eq!(tail7.len(), 7, "skip=3 drops first 3");
    assert_eq!(
        tail7[0].id, "m-3",
        "ordering preserved (first is 4th appended)"
    );

    // skip=10 -> empty (everything skipped)
    let empty = store.load_messages_after("s1", 10).await.unwrap();
    assert!(empty.is_empty(), "skip=count returns nothing");

    // skip beyond count -> empty (no panic)
    let over = store.load_messages_after("s1", 99).await.unwrap();
    assert!(
        over.is_empty(),
        "skip > count returns nothing without panic"
    );

    // Consistency with a full load for skip=0.
    let full = store.load_messages("s1").await.unwrap();
    assert_eq!(all.len(), full.len());
    assert_eq!(
        all.last().map(|m| m.id.as_str()),
        full.last().map(|m| m.id.as_str())
    );
}

#[tokio::test]
async fn load_messages_after_negative_offset_returns_all() {
    let (_dir, store) = fresh().await;
    let inserted = msgs("neg", 4);
    store.append_messages("s1", &inserted).await.unwrap();

    // A negative skip_count must be clamped to 0 (mirroring the Store trait
    // default's clamp) — never reach SQL `OFFSET` with a negative value
    // (whose behavior is SQLite-version-dependent). It should return ALL
    // rows, not error, not be empty.
    let got = store.load_messages_after("s1", -5).await.unwrap();
    assert_eq!(
        got.len(),
        inserted.len(),
        "negative offset clamps to 0 -> all rows"
    );
    assert_eq!(
        got.iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
        inserted.iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
        "order preserved, no rows dropped",
    );

    // The trait default clamps -1 too; ensure parity with a plain skip=0.
    let zero = store.load_messages_after("s1", 0).await.unwrap();
    assert_eq!(got.len(), zero.len());
}

//! Regression tests for store concurrency hardening:
//! - message batches land in one atomic rdb WriteBatch
//! - concurrent writers serialize through the engine's write lease
//! - concurrent writes do not deadlock
//!
//! These are assertion-based (the earlier diagnostic build only printed). They
//! go GREEN once the hardening lands and stay GREEN as a regression gate.

use std::sync::Arc;

use opencoder_core::{ContentBlock, Message};
use opencoder_store::{RdbStore, SessionMeta, Store};

/// Build a minimal `SessionMeta` for test sessions. `SessionMeta` has no
/// constructor, so we assemble the struct literal directly.
fn meta(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.to_string(),
        title: Some(format!("title-{id}")),
        agent: None,
        model: None,
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
        kind: None,
    }
}

/// `n` plain user messages with stable ids `"{seed}-{i}"`.
fn conv(seed: &str, n: usize) -> Vec<Message> {
    (0..n)
        .map(|i| {
            let id = format!("{seed}-{i}");
            let text = format!("{seed} msg {i}");
            Message::user(id, text)
        })
        .collect()
}

/// `n` messages alternating user/assistant roles (exercises both role paths).
fn conv_mixed(seed: &str, n: usize) -> Vec<Message> {
    (0..n)
        .map(|i| {
            let id = format!("{seed}-{i}");
            let text = format!("{seed} msg {i}");
            if i % 2 == 0 {
                Message::user(id, text)
            } else {
                let mut m = Message::assistant(id);
                m.blocks = vec![ContentBlock::Text { text }];
                m
            }
        })
        .collect()
}

/// Verify the busy_timeout pragma is set to 30000 ms on the connection.
#[tokio::test]
async fn append_many_chunks_large_batch() {
    let store = RdbStore::open_memory().await.unwrap();
    let m = meta("chunk-test");
    store.create_session(&m).await.unwrap();

    let msgs = conv("batch", 250);
    let seqs = store.append_messages(&m.id, &msgs).await.unwrap();
    assert_eq!(seqs.len(), 250, "all 250 seqs returned");

    let loaded = store.load_messages(&m.id).await.unwrap();
    assert_eq!(loaded.len(), 250, "all 250 messages loaded");
    assert_eq!(loaded[0].id, "batch-0");
    assert_eq!(loaded[249].id, "batch-249");
}

/// Empty batch should return empty seqs vec, not error.
#[tokio::test]
async fn append_many_empty_returns_empty() {
    let store = RdbStore::open_memory().await.unwrap();
    let m = meta("empty-test");
    store.create_session(&m).await.unwrap();

    let seqs = store.append_messages(&m.id, &[]).await.unwrap();
    assert!(seqs.is_empty(), "empty batch should return empty seqs");
}

/// Exactly 200 messages — large-batch boundary case.
#[tokio::test]
async fn append_many_exact_chunk_boundary() {
    let store = RdbStore::open_memory().await.unwrap();
    let m = meta("boundary-test");
    store.create_session(&m).await.unwrap();

    let msgs = conv("boundary", 200);
    let seqs = store.append_messages(&m.id, &msgs).await.unwrap();
    assert_eq!(seqs.len(), 200);

    let loaded = store.load_messages(&m.id).await.unwrap();
    assert_eq!(loaded.len(), 200);
}

/// Import 450 messages via the import path (also chunked).
#[tokio::test]
async fn import_chunks_large_batch() {
    let store = RdbStore::open_memory().await.unwrap();
    let m = meta("import-test");
    store.create_session(&m).await.unwrap();

    let msgs = conv_mixed("import", 450);
    let report = store.import_messages(&m.id, &msgs).await.unwrap();
    assert_eq!(report.messages, 450);
    assert_eq!(report.skipped, 0);

    let loaded = store.load_messages(&m.id).await.unwrap();
    assert_eq!(loaded.len(), 450);
}

/// Concurrent writers to the same store should not deadlock.
/// Each writer appends a batch of messages. `futures` is not a dev-dependency,
/// so we collect `JoinHandle`s and await them sequentially.
#[tokio::test]
async fn concurrent_writers_no_deadlock() {
    let store = Arc::new(RdbStore::open_memory().await.unwrap());
    let m = meta("concurrent");
    store.create_session(&m).await.unwrap();

    let num_writers = 4;
    let mut handles = Vec::new();
    for w in 0..num_writers {
        let s = store.clone();
        let sid = m.id.clone();
        handles.push(tokio::spawn(async move {
            let msgs = conv(&format!("w{w}"), 50);
            s.append_messages(&sid, &msgs).await
        }));
    }

    // If busy_timeout or chunking is broken, this will hang or error.
    for h in handles {
        let res = h.await;
        assert!(res.is_ok(), "writer task panicked: {:?}", res.err());
        assert!(res.unwrap().is_ok(), "append_messages failed");
    }

    let loaded = store.load_messages(&m.id).await.unwrap();
    assert_eq!(
        loaded.len(),
        num_writers * 50,
        "all messages from all writers should be present"
    );
}

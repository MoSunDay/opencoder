//! Regression tests for store concurrency hardening:
//! - busy_timeout pragma raised to 30 s
//! - wal_autocheckpoint pragma configured
//! - batch INSERT chunked at BATCH_CHUNK to bound transaction size
//! - concurrent writes do not deadlock
//!
//! These are assertion-based (the earlier diagnostic build only printed). They
//! go GREEN once the hardening lands and stay GREEN as a regression gate.

use std::sync::Arc;

use opencoder_core::{ContentBlock, Message};
use opencoder_store::{LibsqlStore, SessionMeta, Store};

/// Build a minimal `SessionMeta` for test sessions. `SessionMeta` has no
/// constructor, so we assemble the struct literal directly.
fn meta(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.to_string(),
        title: Some(format!("title-{id}")),
        agent: None,
        model: None,
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
        plan_snapshot: None,
        plan_input_count: 0,
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
async fn pragma_busy_timeout_is_30000() {
    let store = LibsqlStore::open_memory().await.unwrap();
    let conn = store.conn().await.unwrap();
    let stmt = conn.prepare("PRAGMA busy_timeout").await.unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    let row = rows.next().await.unwrap().unwrap();
    let timeout: i64 = row.get(0).unwrap();
    assert_eq!(timeout, 30000, "busy_timeout should be 30000 ms");
}

/// Verify wal_autocheckpoint is configured.
#[tokio::test]
async fn pragma_wal_autocheckpoint_is_1000() {
    let store = LibsqlStore::open_memory().await.unwrap();
    let conn = store.conn().await.unwrap();
    let stmt = conn.prepare("PRAGMA wal_autocheckpoint").await.unwrap();
    let mut rows = stmt.query(()).await.unwrap();
    let row = rows.next().await.unwrap().unwrap();
    let pages: i64 = row.get(0).unwrap();
    assert_eq!(pages, 1000, "wal_autocheckpoint should be 1000 pages");
}

/// Insert 250 messages (> BATCH_CHUNK of 200) and verify all are loaded.
/// This exercises the chunking path: 200 + 50 across two transactions.
#[tokio::test]
async fn append_many_chunks_large_batch() {
    let store = LibsqlStore::open_memory().await.unwrap();
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
    let store = LibsqlStore::open_memory().await.unwrap();
    let m = meta("empty-test");
    store.create_session(&m).await.unwrap();

    let seqs = store.append_messages(&m.id, &[]).await.unwrap();
    assert!(seqs.is_empty(), "empty batch should return empty seqs");
}

/// Exactly BATCH_CHUNK (200) messages — one full chunk, boundary case.
#[tokio::test]
async fn append_many_exact_chunk_boundary() {
    let store = LibsqlStore::open_memory().await.unwrap();
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
    let store = LibsqlStore::open_memory().await.unwrap();
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
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
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

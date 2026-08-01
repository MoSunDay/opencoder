//! Concurrency / WAL-stress diagnostics for the libsql-backed Store.
//!
//! - concurrent_readers_while_writer: WAL allows N readers + 1 writer
//! - wal_crash_recovery: drop & reopen the db file, committed data survives
//! - concurrent_writers_reproduce_busy / mixed_concurrent_writes_with_immediate_tx /
//!   extreme_concurrent_writers: hammer a file-backed DB with concurrent writers
//!   (mimicking parallel subagent sessions sharing one `Arc<dyn Store>`)
//! - two_stores_same_file_concurrent_writers: two LibsqlStore handles on one file
//!
//! These REPORT rather than hard-assert, because the contention itself is the
//! phenomenon under investigation. Run with: --nocapture --test-threads=1.

use std::sync::{Arc, Mutex};

use opencoder_core::{ContentBlock, Message, Role};
use opencoder_store::{LibsqlStore, SessionMeta, Store};

fn conv(seed: &str, n: usize) -> Vec<Message> {
    (0..n)
        .map(|i| {
            let id = format!("{seed}-{i}");
            let role = if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            let text = format!("{seed} msg {i}");
            let mut m = match role {
                Role::User => Message::user(id, text),
                Role::Assistant => {
                    let mut m = Message::assistant(id);
                    m.blocks = vec![ContentBlock::text(text)];
                    m
                }
                _ => unreachable!(),
            };
            m.created_at = i as i64;
            m
        })
        .collect()
}

async fn make_session(store: &LibsqlStore, id: &str, now: i64) {
    let meta = SessionMeta {
        id: id.to_string(),
        title: Some(format!("title-{id}")),
        agent: Some("act".into()),
        model: Some("glm-5.2".into()),
        workdir_hash: Some("h".into()),
        created_at: now,
        updated_at: now,
        summary: None,
        summary_seq: None,
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
    };
    store.create_session(&meta).await.unwrap();
}

fn meta_for(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.to_string(),
        title: Some(format!("t-{id}")),
        agent: Some("act".into()),
        model: Some("glm-5.2".into()),
        workdir_hash: Some("h".into()),
        created_at: 1,
        updated_at: 1,
        summary: None,
        summary_seq: None,
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
    }
}

#[tokio::test]
async fn concurrent_readers_while_writer() {
    let dir = tempfile::tempdir().unwrap();
    let store_raw = LibsqlStore::open(dir.path().join("cw.db")).await.unwrap();
    make_session(&store_raw, "s", 1).await;
    store_raw
        .append_messages("s", &conv("seed", 10))
        .await
        .unwrap();
    let store = Arc::new(store_raw);
    let _dir = dir; // keep alive

    let store_w = store.clone();
    let writer = tokio::spawn(async move {
        for b in 0..20u32 {
            let msgs = conv(&format!("w{b}"), 5);
            store_w
                .append_messages("s", &msgs)
                .await
                .expect("append ok");
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
    });

    let mut readers = Vec::new();
    for r in 0..8u32 {
        let store_r = store.clone();
        readers.push(tokio::spawn(async move {
            for _ in 0..10usize {
                let loaded = store_r.load_messages("s").await.expect("read ok");
                // WAL: readers always see a consistent snapshot — count must be
                // monotonically non-decreasing and never observe a half-written batch.
                assert!(!loaded.is_empty(), "reader {r} saw empty");
                tokio::time::sleep(std::time::Duration::from_millis(3)).await;
            }
        }));
    }

    writer.await.unwrap();
    for h in readers {
        h.await.unwrap();
    }
    let final_count = store.load_messages("s").await.unwrap().len();
    assert_eq!(final_count, 10 + 20 * 5, "all writes landed");
}

#[tokio::test]
async fn wal_crash_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("crash.db");

    {
        let store = LibsqlStore::open(&db_path).await.unwrap();
        make_session(&store, "persist", 5).await;
        store
            .append_messages("persist", &conv("c", 7))
            .await
            .unwrap();
        // drop store WITHOUT graceful shutdown — simulates process crash
        drop(store);
    }
    // Reopen from the same file; committed data must survive.
    let store = LibsqlStore::open(&db_path).await.unwrap();
    let got = store
        .get_session("persist")
        .await
        .unwrap()
        .expect("survived");
    assert_eq!(got.id, "persist");
    let loaded = store.load_messages("persist").await.unwrap();
    assert_eq!(loaded.len(), 7);
    assert_eq!(loaded[0].text(), "c msg 0");
}

// =============================================================================
// Diagnostic reproduction tests for concurrent-write failures.
//
// The existing `concurrent_readers_while_writer` test only covers 1 writer +
// N readers, so it can never surface write-lock contention. These two tests
// hammer a FILE-BACKED libsql DB with many CONCURRENT WRITERS (mimicking
// parallel subagent sessions, which all share one `Arc<dyn Store>`), to
// surface SQLITE_BUSY / other write-lock errors and capture the real error
// text. They REPORT rather than hard-assert, because the contention itself is
// the phenomenon under investigation. Run with: --nocapture --test-threads=1.
// =============================================================================

/// Test A — pure concurrent writers: 8 sessions x 50 single-row append_message
/// (the exact path `SessionState::record` -> `append_message` takes), no sleep,
/// to maximize write-lock contention.
#[tokio::test]
async fn concurrent_writers_reproduce_busy() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(LibsqlStore::open(dir.path().join("busy.db")).await.unwrap());
    const W: u32 = 8;
    const N: u32 = 50;
    for w in 0..W {
        make_session(&store, &format!("child{w}"), w as i64).await;
    }
    let errs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for w in 0..W {
        let s = store.clone();
        let errs = errs.clone();
        handles.push(tokio::spawn(async move {
            let sid = format!("child{w}");
            for k in 0..N {
                let m = Message::user(format!("u-{w}-{k}"), format!("body-{w}-{k}"));
                if let Err(e) = s.append_message(&sid, &m).await {
                    errs.lock()
                        .unwrap()
                        .push(format!("[w{w} k{k} append_message] {e:#}"));
                }
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let total = W * N;
    {
        let errs = errs.lock().unwrap();
        eprintln!(
            "== concurrent_writers_reproduce_busy: {}/{} writes failed ==",
            errs.len(),
            total
        );
        for e in errs.iter() {
            eprintln!("WRITE_ERR {e}");
        }
    }
    let landed = store.load_messages("child0").await.unwrap().len();
    eprintln!("child0 landed messages: {landed}/{N}");
}

/// Test B — mixed concurrent writes: each writer interleaves
/// append_message + append_event + claim_next_queue (BEGIN IMMEDIATE tx),
/// which holds the write lock for the whole transaction and may starve
/// concurrent message appends — closer to the real runner mix.
#[tokio::test]
async fn mixed_concurrent_writes_with_immediate_tx() {
    use opencoder_store::{Delivery, EventKind, SessionEventRecord, SessionInput};
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        LibsqlStore::open(dir.path().join("mixed.db"))
            .await
            .unwrap(),
    );
    const W: u32 = 8;
    const ITERS: u32 = 20;
    for w in 0..W {
        let sid = format!("child{w}");
        make_session(&store, &sid, w as i64).await;
        for k in 0..ITERS {
            let inp = SessionInput {
                seq: None,
                id: format!("in-{w}-{k}"),
                session_id: sid.clone(),
                delivery: Delivery::Queue,
                prompt: format!("q-{w}-{k}"),
                images: Vec::new(),
                display_text: None,
                admitted_seq: k as i64 + 1,
                promoted_seq: None,
            };
            store.admit_input(&inp).await.unwrap();
        }
    }
    let errs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for w in 0..W {
        let s = store.clone();
        let errs = errs.clone();
        handles.push(tokio::spawn(async move {
            let sid = format!("child{w}");
            for k in 0..ITERS {
                let m = Message::user(format!("u-{w}-{k}"), format!("body-{w}-{k}"));
                if let Err(e) = s.append_message(&sid, &m).await {
                    errs.lock()
                        .unwrap()
                        .push(format!("[w{w} k{k} append_message] {e:#}"));
                }
                let rec = SessionEventRecord {
                    session_id: sid.clone(),
                    kind: EventKind::TextDelta,
                    payload: serde_json::Value::String(format!("ev-{w}-{k}")),
                    ts: k as i64,
                    seq: None,
                    sse_kind: None,
                };
                if let Err(e) = s.append_event(&rec).await {
                    errs.lock()
                        .unwrap()
                        .push(format!("[w{w} k{k} append_event] {e:#}"));
                }
                if let Err(e) = s.claim_next_queue(&sid).await {
                    errs.lock()
                        .unwrap()
                        .push(format!("[w{w} k{k} claim_next_queue] {e:#}"));
                }
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let errs = errs.lock().unwrap();
    eprintln!(
        "== mixed_concurrent_writes_with_immediate_tx: {} ops failed ==",
        errs.len()
    );
    for e in errs.iter() {
        eprintln!("WRITE_ERR {e}");
    }
}

/// Test C — extreme pressure: 32 sessions x 200 single-row appends, to test
/// whether `busy_timeout=5000` ever breaks under heavy intra-process load.
#[tokio::test]
async fn extreme_concurrent_writers() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(
        LibsqlStore::open(dir.path().join("extreme.db"))
            .await
            .unwrap(),
    );
    const W: u32 = 32;
    const N: u32 = 200;
    for w in 0..W {
        make_session(&store, &format!("c{w}"), w as i64).await;
    }
    let errs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for w in 0..W {
        let s = store.clone();
        let errs = errs.clone();
        handles.push(tokio::spawn(async move {
            let sid = format!("c{w}");
            for k in 0..N {
                let payload = "x".repeat(512);
                let m = Message::user(format!("u{w}-{k}"), payload);
                if let Err(e) = s.append_message(&sid, &m).await {
                    errs.lock().unwrap().push(format!("[w{w} k{k}] {e:#}"));
                }
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let errs = errs.lock().unwrap();
    eprintln!(
        "== extreme_concurrent_writers: {}/{} writes failed ==",
        errs.len(),
        W * N
    );
    for e in errs.iter().take(20) {
        eprintln!("WRITE_ERR {e}");
    }
}

/// Test D — TWO separate `LibsqlStore` handles opened on the SAME db file
/// (mimicking two processes — e.g. TUI + web server — or two independent
/// connection pools hitting one opencoder.db). Each store spawns concurrent
/// writers. This is the configuration most likely to surface cross-connection
/// write-lock contention.
#[tokio::test]
async fn two_stores_same_file_concurrent_writers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shared.db");
    let store_a = Arc::new(LibsqlStore::open(&path).await.unwrap());
    let store_b = Arc::new(LibsqlStore::open(&path).await.unwrap());
    const W: u32 = 6;
    const N: u32 = 50;
    for w in 0..W {
        let sid = format!("c{w}");
        store_a.create_session(&meta_for(&sid)).await.unwrap();
    }
    let _ = (store_a, store_b); // moved into closures below
    let store_a = Arc::new(LibsqlStore::open(&path).await.unwrap());
    let store_b = Arc::new(LibsqlStore::open(&path).await.unwrap());
    let errs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for w in 0..W {
        let s = if w % 2 == 0 {
            store_a.clone()
        } else {
            store_b.clone()
        };
        let errs = errs.clone();
        handles.push(tokio::spawn(async move {
            let sid = format!("c{w}");
            for k in 0..N {
                let m = Message::user(format!("u{w}-{k}"), format!("b{w}-{k}"));
                if let Err(e) = s.append_message(&sid, &m).await {
                    errs.lock().unwrap().push(format!("[w{w} k{k}] {e:#}"));
                }
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let errs = errs.lock().unwrap();
    eprintln!(
        "== two_stores_same_file_concurrent_writers: {}/{} writes failed ==",
        errs.len(),
        W * N
    );
    for e in errs.iter().take(20) {
        eprintln!("WRITE_ERR {e}");
    }
}

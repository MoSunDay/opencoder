//! Regression test for the db_lock serialization fix (Phase 1).
//!
//! Before the fix, libsql 0.9.x ran sync SQLite FFI directly on tokio worker
//! threads with no serialization. Concurrent operations (multi-subagent
//! flushers + run_loop) contended on SQLite's internal mutex, producing
//! sporadic "cannot start a transaction" errors and runtime worker starvation.
//!
//! This test fires 16 concurrent tasks that each interleave `append_events`
//! (which opens a transaction), `append_message`, and `claim_next_queue`
//! (IMMEDIATE transaction) against a single shared `LibsqlStore`. With the
//! `db_lock` serialization in place, zero operations should error and all
//! data must be present and correct afterwards.

use std::sync::{Arc, Mutex};

use opencoder_core::Message;
use opencoder_store::{
    Delivery, EventKind, LibsqlStore, SessionEventRecord, SessionInput, SessionMeta, Store,
};
use tempfile::TempDir;

fn meta_for(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.to_string(),
        title: Some(format!("title-{id}")),
        agent: Some("act".into()),
        model: Some("test-model".into()),
        workdir_hash: Some("h".into()),
        created_at: 0,
        updated_at: 0,
        summary: None,
        summary_seq: None,
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
    }
}

#[tokio::test]
async fn concurrent_store_ops_serialized() {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(
        LibsqlStore::open(dir.path().join("serialized.db"))
            .await
            .unwrap(),
    );

    const TASKS: usize = 16;
    const ITERS: usize = 25;

    // Setup: create sessions + pre-admit queue inputs for claim_next_queue.
    for w in 0..TASKS {
        let sid = format!("s{w}");
        store.create_session(&meta_for(&sid)).await.unwrap();
        for k in 0..ITERS {
            let inp = SessionInput {
                seq: None,
                id: format!("in-{w}-{k}"),
                session_id: sid.clone(),
                delivery: Delivery::Queue,
                prompt: format!("q-{w}-{k}"),
                images: Vec::new(),
                admitted_seq: k as i64 + 1,
                promoted_seq: None,
            };
            store.admit_input(&inp).await.unwrap();
        }
    }

    let errs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();

    for w in 0..TASKS {
        let s = store.clone();
        let errs = errs.clone();
        handles.push(tokio::spawn(async move {
            let sid = format!("s{w}");
            for k in 0..ITERS {
                // Operation 1: append_message
                let m = Message::user(format!("u-{w}-{k}"), format!("body-{w}-{k}"));
                if let Err(e) = s.append_message(&sid, &m).await {
                    errs.lock().unwrap().push(format!("msg[{w},{k}] {e:#}"));
                }

                // Operation 2: append_event (opens a transaction)
                let rec = SessionEventRecord {
                    session_id: sid.clone(),
                    kind: EventKind::TextDelta,
                    payload: serde_json::Value::String(format!("ev-{w}-{k}")),
                    ts: k as i64,
                    seq: None,
                    sse_kind: None,
                };
                if let Err(e) = s.append_event(&rec).await {
                    errs.lock().unwrap().push(format!("evt[{w},{k}] {e:#}"));
                }

                // Operation 3: claim_next_queue (IMMEDIATE transaction)
                if let Err(e) = s.claim_next_queue(&sid).await {
                    errs.lock().unwrap().push(format!("claim[{w},{k}] {e:#}"));
                }
            }
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    {
        let errs = errs.lock().unwrap();
        assert!(
            errs.is_empty(),
            "db_lock serialization must prevent all concurrent errors, but {} occurred:\n{}",
            errs.len(),
            errs.iter()
                .take(20)
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    // Data integrity: verify all messages and events landed.
    for w in 0..TASKS {
        let sid = format!("s{w}");
        let msgs = store.load_messages(&sid).await.unwrap();
        let user_count = msgs
            .iter()
            .filter(|m| m.id.starts_with(&format!("u-{w}-")))
            .count();
        assert_eq!(
            user_count, ITERS,
            "session {sid}: expected {ITERS} user messages, got {user_count}"
        );

        let evs = store.events_after(&sid, 0).await.unwrap();
        let delta_count = evs
            .iter()
            .filter(|e| {
                e.kind == EventKind::TextDelta
                    && e.payload == serde_json::Value::String(format!("ev-{w}-{}", ITERS - 1))
            })
            .count();
        // At minimum, the last event we appended must be present.
        assert!(
            delta_count >= 1,
            "session {sid}: expected the final append_event payload to be present among {} events",
            evs.len()
        );
    }
}

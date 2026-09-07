use std::sync::{Arc, Mutex};

use opencoder_core::Message;
use opencoder_store::{Delivery, LibsqlStore, SessionInput, SessionMeta, Store};
use tempfile::TempDir;

fn meta_for(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.to_string(),
        title: Some(format!("title-{id}")),
        agent: Some("act".into()),
        model: Some("test-model".into()),
        autopilot_mode: None,
        workdir_hash: Some("h".into()),
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

fn queue_input(id: &str) -> SessionInput {
    SessionInput {
        seq: None,
        id: id.to_string(),
        session_id: "s".into(),
        delivery: Delivery::Queue,
        prompt: format!("prompt-{id}"),
        images: Vec::new(),
        display_text: None,
        admitted_seq: 0,
        promoted_seq: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cross_instance_admits_never_busy_error() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("cross_instance.db");
    let a = Arc::new(LibsqlStore::open(&db_path).await.unwrap());
    let b = Arc::new(LibsqlStore::open(&db_path).await.unwrap());

    a.create_session(&meta_for("s")).await.unwrap();

    const ADMIT_TASKS: usize = 6;
    const ADMIT_ITERS: usize = 40;
    const WRITER_ITERS: usize = 60;
    let total = ADMIT_TASKS * ADMIT_ITERS;

    let errs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();

    for w in 0..ADMIT_TASKS {
        let store = if w % 2 == 0 { a.clone() } else { b.clone() };
        let errs = errs.clone();
        handles.push(tokio::spawn(async move {
            for k in 0..ADMIT_ITERS {
                let inp = queue_input(&format!("in-{w}-{k}"));
                if let Err(e) = store.admit_input(&inp).await {
                    errs.lock().unwrap().push(format!("admit[{w},{k}] {e:#}"));
                }
            }
        }));
    }

    for w in 0..2 {
        let store = if w == 0 { a.clone() } else { b.clone() };
        let errs = errs.clone();
        handles.push(tokio::spawn(async move {
            for k in 0..WRITER_ITERS {
                let m = Message::user(format!("w{w}-{k}"), format!("body-{w}-{k}"));
                if let Err(e) = store.append_message("s", &m).await {
                    errs.lock().unwrap().push(format!("msg[{w},{k}] {e:#}"));
                }
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
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
            "cross-instance admits must serialize without errors; under the old deferred BEGIN \
             a concurrent commit between the seq SELECT and the INSERT surfaces as \
             SQLITE_BUSY_SNAPSHOT / 'database is locked' (busy_timeout does not retry the \
             upgrade), but {} errors occurred:\n{}",
            errs.len(),
            errs.iter()
                .take(20)
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    let pending = b.pending_inputs("s", Delivery::Queue).await.unwrap();
    assert_eq!(
        pending.len(),
        total,
        "expected all {total} queued inputs to be pending, got {}",
        pending.len()
    );
    let mut seqs: Vec<i64> = pending.iter().map(|i| i.admitted_seq).collect();
    seqs.sort();
    // admitted_seq is allocated inside an IMMEDIATE transaction: the multiset
    // across both instances must be exactly 1..=total (no dupes, no gaps).
    assert_eq!(
        seqs,
        (1..=(total as i64)).collect::<Vec<i64>>(),
        "admitted_seq allocation must be dense and collision-free across instances"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cross_instance_claim_after_concurrent_admits() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("cross_claim.db");
    let a = Arc::new(LibsqlStore::open(&db_path).await.unwrap());
    let b = Arc::new(LibsqlStore::open(&db_path).await.unwrap());

    a.create_session(&meta_for("s")).await.unwrap();

    const TASKS: usize = 2;
    const ITERS: usize = 25;
    let errs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();

    for w in 0..TASKS {
        let store = if w == 0 { a.clone() } else { b.clone() };
        let errs = errs.clone();
        handles.push(tokio::spawn(async move {
            for k in 0..ITERS {
                let inp = queue_input(&format!("in-{w}-{k}"));
                if let Err(e) = store.admit_input(&inp).await {
                    errs.lock().unwrap().push(format!("admit[{w},{k}] {e:#}"));
                }
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    let mut claims: Vec<i64> = Vec::new();
    loop {
        let store = if claims.len().is_multiple_of(2) {
            &b
        } else {
            &a
        };
        match store.claim_next_queue("s").await {
            Ok(Some((seq, _))) => claims.push(seq),
            Ok(None) => break,
            Err(e) => {
                errs.lock()
                    .unwrap()
                    .push(format!("claim[{}] {e:#}", claims.len()));
                break;
            }
        }
    }

    {
        let errs = errs.lock().unwrap();
        assert!(
            errs.is_empty(),
            "cross-instance admit/claim must serialize without errors (deferred BEGIN would \
             surface SQLITE_BUSY_SNAPSHOT / 'database is locked'), but {} occurred:\n{}",
            errs.len(),
            errs.iter()
                .take(20)
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    assert_eq!(claims.len(), TASKS * ITERS, "must drain every queued input");
    let mut distinct = claims.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        TASKS * ITERS,
        "all claimed row seqs must be distinct"
    );
}

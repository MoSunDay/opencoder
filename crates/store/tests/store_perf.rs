//! P5 performance threshold tests (run with --release for representative numbers).
//! These assert real latency contracts, not "it runs":
//! - 1000 message appends average < 2ms each (store_append_p99_under_2ms)
//! - loading 1000 messages < 50ms (load throughput)
//! - listing 200 sessions < 100ms

use std::time::Instant;

use opencode_core::{ContentBlock, Message, Role};
use opencode_store::{LibsqlStore, SessionFilter, SessionMeta, Store};
use tempfile::TempDir;

async fn setup() -> (TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("perf.db")).await.unwrap();
    (dir, store)
}

fn bulk(seed: &str, n: usize) -> Vec<Message> {
    (0..n)
        .map(|i| {
            let id = format!("{seed}-{i}");
            let mut m = Message::user(id, format!("{seed} body {i}").repeat(3));
            m.created_at = i as i64;
            m
        })
        .collect()
}

#[tokio::test]
async fn append_1000_messages_under_2ms_avg() {
    let (_dir, store) = setup().await;
    store
        .create_session(&SessionMeta {
            id: "p".into(),
            title: None,
            agent: None,
            model: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
        })
        .await
        .unwrap();
    let msgs = bulk("m", 1000);
    let t = Instant::now();
    let seqs = store.append_messages("p", &msgs).await.unwrap();
    let elapsed = t.elapsed();
    assert_eq!(seqs.len(), 1000);
    let per = elapsed.as_secs_f64() / 1000.0 * 1000.0;
    eprintln!("append 1000 msgs: {:.2?} ({:.3} ms/append)", elapsed, per);
    assert!(per < 2.0, "append avg must be < 2ms, got {per:.3}ms");
}

#[tokio::test]
async fn load_1000_messages_under_50ms() {
    let (_dir, store) = setup().await;
    store
        .create_session(&SessionMeta {
            id: "p".into(),
            title: None,
            agent: None,
            model: None,
            workdir_hash: None,
            created_at: 0,
            updated_at: 0,
            summary: None,
            summary_seq: None,
        })
        .await
        .unwrap();
    store.append_messages("p", &bulk("m", 1000)).await.unwrap();
    let t = Instant::now();
    let loaded = store.load_messages("p").await.unwrap();
    let elapsed = t.elapsed();
    assert_eq!(loaded.len(), 1000);
    eprintln!("load 1000 msgs: {:.2?}", elapsed);
    assert!(
        elapsed.as_millis() < 50,
        "load 1000 must be < 50ms, got {:?}",
        elapsed
    );
}

#[tokio::test]
async fn list_200_sessions_under_100ms() {
    let (_dir, store) = setup().await;
    for i in 0..200u32 {
        let id = format!("s{i}");
        store
            .create_session(&SessionMeta {
                id: id.clone(),
                title: Some(format!("t{i}")),
                agent: Some("act".into()),
                model: Some("m".into()),
                workdir_hash: None,
                created_at: i as i64,
                updated_at: i as i64,
                summary: None,
                summary_seq: None,
            })
            .await
            .unwrap();
        store.append_messages(&id, &bulk(&id, 1)).await.unwrap();
    }
    let t = Instant::now();
    let items = store
        .list_sessions(&SessionFilter {
            limit: 200,
            ..Default::default()
        })
        .await
        .unwrap();
    let elapsed = t.elapsed();
    assert_eq!(items.len(), 200);
    eprintln!("list 200 sessions (with preview subquery): {:.2?}", elapsed);
    assert!(
        elapsed.as_millis() < 100,
        "list 200 must be < 100ms, got {:?}",
        elapsed
    );
    // keep ContentBlock import honest
    let _ = ContentBlock::text("x");
    let _ = Role::User;
}

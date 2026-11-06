//! Durable queue ledger behavior over the rdb (RocksDB) backend.
//!
//! The rdb wrapper has no in-memory mode, so every test opens a database
//! under a fresh tempfile directory; reopen cases reuse the same path.

#![cfg(feature = "rdb")]

use chrono::{TimeZone, Utc};
use opencoder_event::domain::event::{rebuild_record, EventKind, EXECUTION_PAUSED};
use opencoder_event::domain::listener::{new_listener, TriggerAction};
use opencoder_event::ledger::EventLedger;
use opencoder_event::queue::RdbQueueLedger;
use serde_json::json;

fn record(id: &str, kind: &str, seq: i64) -> opencoder_event::domain::event::EventRecord {
    let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    rebuild_record(
        id,
        EventKind::new(kind),
        json!({"seq": seq}),
        "test",
        seq,
        now,
    )
}

fn notify_listener(id: &str, pattern: &str) -> opencoder_event::domain::listener::ListenerSpec {
    new_listener(
        id,
        pattern,
        None,
        TriggerAction::Notify {
            message: "m".to_string(),
        },
    )
}

#[tokio::test]
async fn append_assigns_monotonic_seqs_and_reads_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = RdbQueueLedger::open(dir.path().join("db")).unwrap();

    // Caller seqs arrive out of order; the ledger owns ordering.
    ledger.append(&record("a", "event.custom.c", 7)).await.unwrap();
    ledger.append(&record("b", "event.custom.a", 2)).await.unwrap();
    ledger.append(&record("c", "event.custom.b", 40)).await.unwrap();

    let all = ledger.events_after(0, 10).await.unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(
        all.iter().map(|r| r.seq).collect::<Vec<_>>(),
        [1, 2, 3],
        "seqs are allocated strictly increasing in append order"
    );
    assert_eq!(
        all.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["a", "b", "c"],
        "rows keep append order, not the caller's seq"
    );
    assert_eq!(ledger.last_seq().await.unwrap(), 3);

    // strictly-after-cursor reads and the limit cap
    let after_two = ledger.events_after(2, 10).await.unwrap();
    assert_eq!(after_two.iter().map(|r| r.seq).collect::<Vec<_>>(), [3]);
    let paged = ledger.events_after(0, 2).await.unwrap();
    assert_eq!(paged.iter().map(|r| r.seq).collect::<Vec<_>>(), [1, 2]);
    assert!(ledger.events_after(3, 10).await.unwrap().is_empty());
    assert!(ledger.events_after(0, 0).await.unwrap().is_empty());
}

#[tokio::test]
async fn append_rejects_blank_id_and_kind() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = RdbQueueLedger::open(dir.path().join("db")).unwrap();
    assert!(ledger.append(&record("", "event.custom.x", 1)).await.is_err());
    assert!(ledger.append(&record("id", "  ", 1)).await.is_err());
    assert_eq!(ledger.last_seq().await.unwrap(), 0);
}

#[tokio::test]
async fn duplicate_append_yields_two_log_rows() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = RdbQueueLedger::open(dir.path().join("db")).unwrap();
    let same = record("dup", "event.custom.tick", 1);
    ledger.append(&same).await.unwrap();
    ledger.append(&same).await.unwrap();
    let rows = ledger.events_after(0, 10).await.unwrap();
    assert_eq!(rows.iter().map(|r| r.seq).collect::<Vec<_>>(), [1, 2]);
    assert!(rows.windows(2).all(|w| w[0].id == w[1].id));
}

#[tokio::test]
async fn log_and_cursors_survive_reopen_with_monotonic_counter() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");

    let ledger = RdbQueueLedger::open(&path).unwrap();
    for seq in 1..=3 {
        ledger
            .append(&record(&format!("e{seq}"), "event.custom.tick", 100 + seq))
            .await
            .unwrap();
    }
    ledger.save_consumer_cursor("pump-a", 7).await.unwrap();
    ledger.save_consumer_cursor("pump-b", 3).await.unwrap();
    drop(ledger);

    let ledger = RdbQueueLedger::open(&path).unwrap();
    let replayed = ledger.events_after(0, 10).await.unwrap();
    assert_eq!(
        replayed.iter().map(|r| r.seq).collect::<Vec<_>>(),
        [1, 2, 3],
        "log rows and their seqs survive the reopen"
    );
    assert_eq!(ledger.consumer_cursor("pump-a").await.unwrap(), Some(7));
    assert_eq!(ledger.consumer_cursor("pump-b").await.unwrap(), Some(3));

    // The seq counter survived too: the next append continues, never restarts.
    ledger.append(&record("e4", "event.custom.tick", 1)).await.unwrap();
    assert_eq!(ledger.last_seq().await.unwrap(), 4);
    let tail = ledger.events_after(3, 10).await.unwrap();
    assert_eq!(tail.iter().map(|r| r.seq).collect::<Vec<_>>(), [4]);

    // Moving one consumer forward overwrites only its own row.
    ledger.save_consumer_cursor("pump-a", 9).await.unwrap();
    assert_eq!(ledger.consumer_cursor("pump-a").await.unwrap(), Some(9));
    assert_eq!(ledger.consumer_cursor("pump-b").await.unwrap(), Some(3));
}

#[tokio::test]
async fn unsafe_consumer_ids_and_negative_cursors_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = RdbQueueLedger::open(dir.path().join("db")).unwrap();
    assert_eq!(ledger.consumer_cursor("pump").await.unwrap(), None);

    let too_long = "a".repeat(opencoder_event::domain::consumer::MAX_CONSUMER_ID_BYTES + 1);
    for bad in [
        "",
        "id with space",
        "a/b",
        "\tid",
        "\u{1}",
        ".",
        "..",
        too_long.as_str(),
    ] {
        assert!(ledger.save_consumer_cursor(bad, 1).await.is_err(), "{bad:?}");
        assert!(ledger.consumer_cursor(bad).await.is_err(), "{bad:?}");
    }
    assert!(ledger.save_consumer_cursor("pump", -1).await.is_err());
    assert_eq!(ledger.consumer_cursor("pump").await.unwrap(), None);
}

#[tokio::test]
async fn second_open_of_a_live_path_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let owner = RdbQueueLedger::open(&path).unwrap();
    assert!(
        RdbQueueLedger::open(&path).is_err(),
        "the database lock must enforce a single owner"
    );
    drop(owner);
    assert!(RdbQueueLedger::open(&path).is_ok());
}

#[tokio::test]
async fn listener_crud_is_durable_and_validates_specs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");

    let ledger = RdbQueueLedger::open(&path).unwrap();
    ledger
        .save_listener(&notify_listener("l2", "brain.*"))
        .await
        .unwrap();
    ledger
        .save_listener(&notify_listener("l1", "execution.*"))
        .await
        .unwrap();

    let listed = ledger.list_listeners().await.unwrap();
    assert_eq!(
        listed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        ["l1", "l2"],
        "listeners list sorted by id"
    );

    // "execution.**" has two stars; "" is not a valid listener id.
    assert!(ledger
        .save_listener(&notify_listener("l3", "execution.**"))
        .await
        .is_err());
    assert!(ledger
        .save_listener(&notify_listener("", "execution.*"))
        .await
        .is_err());

    ledger.delete_listener("l1").await.unwrap();
    ledger.delete_listener("missing").await.unwrap(); // idempotent
    assert_eq!(
        ledger
            .list_listeners()
            .await
            .unwrap()
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["l2"]
    );
    drop(ledger);

    let ledger = RdbQueueLedger::open(&path).unwrap();
    assert_eq!(
        ledger
            .list_listeners()
            .await
            .unwrap()
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["l2"],
        "listener rows survive the reopen"
    );
}

#[tokio::test]
async fn waits_match_exact_and_glob_and_clear_idempotently() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = RdbQueueLedger::open(dir.path().join("db")).unwrap();

    ledger.save_wait("exec-a", EXECUTION_PAUSED).await.unwrap();
    ledger.save_wait("exec-b", "execution.*").await.unwrap();

    let due = ledger
        .due_waits(&EventKind::new(EXECUTION_PAUSED))
        .await
        .unwrap();
    assert_eq!(due, ["exec-a", "exec-b"], "exact plus glob, ordered");
    assert!(ledger
        .due_waits(&EventKind::new("brain.operation_terminal"))
        .await
        .unwrap()
        .is_empty());

    // Re-saving replaces the pattern.
    ledger.save_wait("exec-a", "execution.terminal").await.unwrap();
    let due = ledger
        .due_waits(&EventKind::new(EXECUTION_PAUSED))
        .await
        .unwrap();
    assert_eq!(due, ["exec-b"]);

    // Invalid patterns and blank execution ids are rejected.
    assert!(ledger.save_wait("exec-c", "execution.*.paused").await.is_err());
    assert!(ledger.save_wait("   ", "execution.*").await.is_err());

    // Clearing is idempotent.
    ledger.clear_wait("exec-b").await.unwrap();
    ledger.clear_wait("exec-b").await.unwrap();
    assert!(ledger
        .due_waits(&EventKind::new(EXECUTION_PAUSED))
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        ledger
            .due_waits(&EventKind::new("execution.terminal"))
            .await
            .unwrap(),
        ["exec-a"]
    );
}

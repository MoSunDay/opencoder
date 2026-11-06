//! Bus fan-out behavior and MemoryLedger reference semantics.

use chrono::{TimeZone, Utc};
use opencoder_event::bus::EventBus;
use opencoder_event::domain::event::{new_record, EventKind};
use opencoder_event::domain::listener::{new_listener, TriggerAction};
use opencoder_event::ledger::{EventLedger, MemoryLedger};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

fn record(seq: i64, kind: &str) -> opencoder_event::domain::event::EventRecord {
    let now = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    new_record(EventKind::new(kind), json!({"seq": seq}), "test", seq, now)
}

#[tokio::test]
async fn publish_fans_out_to_every_subscriber() {
    let bus = EventBus::new(16);
    let mut first = bus.subscribe();
    let mut second = bus.subscribe();
    assert_eq!(bus.subscriber_count(), 2);

    for seq in 1..=3 {
        assert_eq!(bus.publish(&record(seq, "execution.paused")), 2);
    }
    for seq in 1..=3 {
        for rx in [&mut first, &mut second] {
            let got = rx.recv().await.expect("subscriber receives");
            assert_eq!(got.seq, seq);
            assert_eq!(got.kind.as_str(), "execution.paused");
            assert_eq!(got.payload["seq"], seq);
        }
    }
}

#[tokio::test]
async fn publish_without_subscribers_returns_zero() {
    let bus = EventBus::new(4);
    assert_eq!(bus.publish(&record(1, "event.custom.noop")), 0);
    let _held = bus.subscribe();
    assert_eq!(bus.publish(&record(2, "event.custom.noop")), 1);
    assert_eq!(bus.subscriber_count(), 1);
}

#[tokio::test]
async fn laggy_subscriber_does_not_block_fast_subscriber() {
    let bus = EventBus::new(2);
    let mut fast = bus.subscribe();
    let mut slow = bus.subscribe();

    assert_eq!(bus.publish(&record(1, "event.custom.tick")), 2);
    assert_eq!(fast.recv().await.unwrap().seq, 1);

    // slow never reads: only the newest two records stay retained.
    for seq in 2..=5 {
        assert_eq!(bus.publish(&record(seq, "event.custom.tick")), 2);
    }

    // fast fell behind by more than capacity: it lags but still reads the tail.
    assert!(matches!(fast.recv().await, Err(RecvError::Lagged(_))));
    assert_eq!(fast.recv().await.unwrap().seq, 4);
    assert_eq!(fast.recv().await.unwrap().seq, 5);

    // slow lagged as well, yet the bus served fast the whole time.
    assert!(matches!(slow.recv().await, Err(RecvError::Lagged(_))));
    assert_eq!(slow.recv().await.unwrap().seq, 4);
    assert_eq!(slow.recv().await.unwrap().seq, 5);

    // fresh traffic still flows after the lag.
    assert_eq!(bus.publish(&record(6, "event.custom.tick")), 2);
    assert_eq!(fast.recv().await.unwrap().seq, 6);
}

#[tokio::test]
async fn events_after_orders_by_seq_and_honors_cursor_and_limit() {
    let ledger = MemoryLedger::new();
    // appended out of order to prove read-side ordering
    ledger.append(&record(2, "event.custom.a")).await.unwrap();
    ledger.append(&record(1, "event.custom.b")).await.unwrap();
    ledger.append(&record(3, "event.custom.c")).await.unwrap();

    let all = ledger.events_after(-1, 10).await.unwrap();
    let seqs: Vec<i64> = all.iter().map(|r| r.seq).collect();
    assert_eq!(seqs, [1, 2, 3]);
    assert_eq!(all[0].kind.as_str(), "event.custom.b");
    assert_eq!(all[0].source, "test");

    let tail: Vec<i64> = ledger
        .events_after(1, 10)
        .await
        .unwrap()
        .iter()
        .map(|r| r.seq)
        .collect();
    assert_eq!(tail, [2, 3]);

    let limited: Vec<i64> = ledger
        .events_after(-1, 2)
        .await
        .unwrap()
        .iter()
        .map(|r| r.seq)
        .collect();
    assert_eq!(limited, [1, 2]);

    assert!(ledger.events_after(-1, 0).await.unwrap().is_empty());
    assert!(ledger.events_after(3, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn listener_crud_persists_upserts_and_validates() {
    let ledger = MemoryLedger::new();
    ledger
        .save_listener(&new_listener(
            "l1",
            "execution.*",
            None,
            TriggerAction::Notify {
                message: "first".to_string(),
            },
        ))
        .await
        .unwrap();
    ledger
        .save_listener(&new_listener(
            "l2",
            "brain.*",
            Some("scope-a".to_string()),
            TriggerAction::CancelExecution {
                execution_id: "exec-1".to_string(),
            },
        ))
        .await
        .unwrap();

    let listed = ledger.list_listeners().await.unwrap();
    let ids: Vec<&str> = listed.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(ids, ["l1", "l2"]);
    assert_eq!(listed[1].scope.as_deref(), Some("scope-a"));

    // upsert: same id replaces the stored spec
    ledger
        .save_listener(&new_listener(
            "l1",
            "execution.terminal",
            None,
            TriggerAction::Notify {
                message: "second".to_string(),
            },
        ))
        .await
        .unwrap();
    let listed = ledger.list_listeners().await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].kind_pattern, "execution.terminal");

    // invalid specs rejected, registry untouched
    let bad_pattern = new_listener(
        "l3",
        "execution.*.paused",
        None,
        TriggerAction::Notify {
            message: "m".to_string(),
        },
    );
    assert!(ledger.save_listener(&bad_pattern).await.is_err());
    let bad_trigger = new_listener(
        "l4",
        "execution.*",
        None,
        TriggerAction::CancelExecution {
            execution_id: String::new(),
        },
    );
    assert!(ledger.save_listener(&bad_trigger).await.is_err());
    assert_eq!(ledger.list_listeners().await.unwrap().len(), 2);

    // delete: known id removed, unknown id is a no-op success
    ledger.delete_listener("missing").await.unwrap();
    ledger.delete_listener("l2").await.unwrap();
    let remaining = ledger.list_listeners().await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, "l1");
}

#[tokio::test]
async fn wait_semantics_match_exact_and_glob_and_clear() {
    let ledger = MemoryLedger::new();
    ledger
        .save_wait("exec-a", "execution.paused")
        .await
        .unwrap();
    ledger.save_wait("exec-b", "execution.*").await.unwrap();
    ledger
        .save_wait("exec-c", "brain.operation_terminal")
        .await
        .unwrap();

    // exact wait plus prefix-glob wait, ordered by execution id
    let due_paused = ledger
        .due_waits(&EventKind::new("execution.paused"))
        .await
        .unwrap();
    assert_eq!(due_paused, ["exec-a", "exec-b"]);

    let due_terminal = ledger
        .due_waits(&EventKind::new("brain.operation_terminal"))
        .await
        .unwrap();
    assert_eq!(due_terminal, ["exec-c"]);

    assert!(ledger
        .due_waits(&EventKind::new("brain.layer_barrier_reached"))
        .await
        .unwrap()
        .is_empty());

    // re-saving a wait replaces its pattern
    ledger
        .save_wait("exec-a", "execution.terminal")
        .await
        .unwrap();
    let due = ledger
        .due_waits(&EventKind::new("execution.paused"))
        .await
        .unwrap();
    assert_eq!(due, ["exec-b"]);

    // clearing is idempotent
    ledger.clear_wait("exec-b").await.unwrap();
    ledger.clear_wait("exec-b").await.unwrap();
    assert!(ledger
        .due_waits(&EventKind::new("execution.paused"))
        .await
        .unwrap()
        .is_empty());

    // invalid patterns and blank execution ids rejected; the overwritten
    // exec-a wait still matches its new pattern
    assert!(ledger
        .save_wait("exec-d", "execution.*.paused")
        .await
        .is_err());
    assert!(ledger.save_wait("   ", "execution.*").await.is_err());
    assert_eq!(
        ledger
            .due_waits(&EventKind::new("execution.terminal"))
            .await
            .unwrap(),
        ["exec-a"]
    );
}

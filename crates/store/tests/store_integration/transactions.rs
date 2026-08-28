//! Transaction atomicity and cancellation-safety contracts.

use std::sync::Arc;

use crate::common::{conv, fresh, make_session};
use opencoder_store::Store;

#[tokio::test]
async fn transaction_rollback_on_partial_failure() {
    let (_dir, store) = fresh().await;
    make_session(&store, "ok", 1).await;

    // Atomicity contract: appending to a non-existent session (FK violation)
    // fails and leaves NO partial state for that session.
    let bad = store.append_messages("ghost-session", &conv("g", 3)).await;
    assert!(bad.is_err(), "FK violation must error");
    assert!(store
        .load_messages("ghost-session")
        .await
        .unwrap()
        .is_empty());

    // The legit session is unaffected.
    store.append_messages("ok", &conv("ok", 2)).await.unwrap();
    assert_eq!(store.load_messages("ok").await.unwrap().len(), 2);

    // Mid-tx rollback at the libsql level: 3 valid inserts followed by a
    // NOT-NULL violation must roll back ALL of them.
    let conn = store.conn().await.unwrap();
    let tx = conn.transaction().await.unwrap();
    tx.execute(
        "INSERT INTO messages (id, session_id, role, blocks_json, usage_json, created_at, synthetic) VALUES ('r1','ok','user','[]','{}',1,0)",
        libsql::params![],
    )
    .await
    .unwrap();
    tx.execute(
        "INSERT INTO messages (id, session_id, role, blocks_json, usage_json, created_at, synthetic) VALUES ('r2','ok','user','[]','{}',2,0)",
        libsql::params![],
    )
    .await
    .unwrap();
    let failed = tx
        .execute(
            "INSERT INTO messages (id, session_id, role, blocks_json, usage_json, created_at, synthetic) VALUES (NULL,'ok','user','[]','{}',3,0)",
            libsql::params![],
        )
        .await;
    assert!(failed.is_err(), "NOT NULL violation must error");
    drop(tx); // explicit drop = rollback
              // none of r1/r2 landed
    let loaded = store.load_messages("ok").await.unwrap();
    assert_eq!(loaded.len(), 2, "rolled-back rows must not appear");
}

// ---------------------------------------------------------------------------
// Regression: future cancellation must not panic (no libsql::Transaction::Drop)
// ---------------------------------------------------------------------------
//
// Before the fix, every transaction used `libsql::Transaction` whose `Drop`
// calls `do_rollback().unwrap()`. When a future was cancelled mid-transaction
// (e.g. via `tokio::select!`), the `db_lock` guard could be released before
// the `Transaction` was dropped, allowing another task to mutate the shared
// connection and invalidate the transaction state -- causing the Drop's
// `unwrap()` to panic the entire process.
//
// With manual BEGIN/COMMIT/ROLLBACK (run_tx), cancellation leaves at worst a
// dangling transaction that the next run_tx recovers from via a pre-BEGIN
// ROLLBACK. No panic, no crash, no data corruption.

#[tokio::test]
async fn cancelled_transaction_does_not_panic() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s1", 1).await;

    // Start a multi-message append (opens a transaction) and cancel it after
    // a tiny delay -- simulating tokio::select! interrupting a drain step.
    let big_batch = conv("cancel", 50);
    let store = Arc::new(store);

    let cancelled = {
        let s = store.clone();
        tokio::select! {
            // Bias toward the timeout so the append future starts but gets
            // dropped before (or shortly after) it can commit.
            _ = tokio::time::sleep(std::time::Duration::from_millis(1)) => true,
            res = s.append_messages("s1", &big_batch) => {
                // If it managed to commit, that's fine too -- the point is no
                // panic.
                let _ = res;
                false
            }
        }
    };

    // Regardless of whether the cancelled batch committed, the store MUST be
    // usable afterwards without panicking or erroring.
    let follow_up = conv("after", 3);
    store.append_messages("s1", &follow_up).await.unwrap();

    // The follow-up messages must be present and correct.
    let loaded = store.load_messages("s1").await.unwrap();
    let _ = cancelled; // unused in assertions -- we just needed the drop to happen
    assert!(
        loaded.iter().any(|m| m.id == "after-0"),
        "post-cancellation append must be persisted"
    );
    // If the cancelled batch committed, there may be up to 50 + 3 = 53 rows.
    // If it was dropped mid-transaction, the dangling tx is rolled back by
    // the next run_tx, so only the 3 follow-up rows exist. Either way, the
    // 3 follow-up messages must all be present.
    for i in 0..3 {
        let id = format!("after-{i}");
        assert!(
            loaded.iter().any(|m| m.id == id),
            "follow-up message {id} must be present"
        );
    }
}

#[tokio::test]
async fn cancelled_then_concurrent_ops_stay_consistent() {
    // Stress variant: cancel several transaction futures interleaved with
    // successful operations, then verify final state is consistent.
    let (_dir, store) = fresh().await;
    make_session(&store, "sx", 1).await;
    let store = Arc::new(store);

    const ROUNDS: usize = 10;

    for round in 0..ROUNDS {
        // Cancel a batch.
        let batch = conv(&format!("c{round}"), 5);
        let s = store.clone();
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_micros(100)) => {}
            _ = s.append_messages("sx", &batch) => {}
        }

        // Immediately do a successful append -- must not panic.
        let ok = conv(&format!("ok{round}"), 1);
        store.append_messages("sx", &ok).await.unwrap();
    }

    // All 10 "ok" messages must be present.
    let loaded = store.load_messages("sx").await.unwrap();
    for round in 0..ROUNDS {
        let id = format!("ok{round}-0");
        assert!(
            loaded.iter().any(|m| m.id == id),
            "ok message {id} from round {round} must survive"
        );
    }
}

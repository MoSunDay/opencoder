//! Input FIFO contracts for the rdb backend, ported from the SQL suites:
//! the recorded state machine, atomic idempotent admission, the FIFO
//! single-active claim guard under concurrency, and queue reordering.

use super::*;
use opencoder_store::{Delivery, InputConflict};

fn steer(session: &str, id: &str) -> SessionInput {
    let mut i = queue_input(session, id);
    i.delivery = Delivery::Steer;
    i
}

#[tokio::test]
async fn recorded_state_machine_pending_promote_mark() {
    let store = mem().await;
    store.create_session(&meta_for("s", 1)).await.unwrap();

    let seq = store.admit_input(&steer("s", "in-1")).await.unwrap();
    // Admitted: pending (not promoted, not recorded), FIFO position 1.
    let pending = store.pending_inputs("s", Delivery::Steer).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].seq, Some(seq));
    assert_eq!(pending[0].admitted_seq, 1);
    assert_eq!(pending[0].promoted_seq, None);

    // Promote: promoted_seq set, consumption marker still clear.
    let promoted = store.promote_inputs("s", 1, Delivery::Steer).await.unwrap();
    assert_eq!(promoted, vec![seq]);
    assert!(store
        .pending_inputs("s", Delivery::Steer)
        .await
        .unwrap()
        .is_empty());

    // Record: durably consumed; marking is idempotent (empty slice too).
    store.mark_inputs_recorded("s", &[seq]).await.unwrap();
    store.mark_inputs_recorded("s", &[]).await.unwrap();
    store.mark_inputs_recorded("s", &[seq]).await.unwrap();
    assert!(store
        .pending_inputs("s", Delivery::Steer)
        .await
        .unwrap()
        .is_empty());
    // Other delivery lanes are unaffected.
    assert!(store
        .pending_inputs("s", Delivery::Queue)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn promote_resets_recorded_and_unpromote_keeps_it() {
    let store = mem().await;
    store.create_session(&meta_for("s", 1)).await.unwrap();
    let seq = store.admit_input(&steer("s", "in-1")).await.unwrap();
    store.promote_inputs("s", 1, Delivery::Steer).await.unwrap();
    store.mark_inputs_recorded("s", &[seq]).await.unwrap();

    // Unpromote returns the row to pending but keeps the recorded marker:
    // only promote itself resets it.
    store.unpromote_inputs("s", &[seq]).await.unwrap();
    let pending = store.pending_inputs("s", Delivery::Steer).await.unwrap();
    assert_eq!(pending.len(), 1, "unpromoted row re-enters pending");
    assert_eq!(pending[0].promoted_seq, None);
    // Recorded rows are never re-served (single-consume).
    assert!(store.claim_next_queue("s").await.unwrap().is_none());

    // Re-promote resets the stale consume marker.
    store.promote_inputs("s", 1, Delivery::Steer).await.unwrap();
    store.unpromote_inputs("s", &[seq]).await.unwrap();
    store.unpromote_inputs("s", &[seq]).await.unwrap(); // idempotent
    let pending = store.pending_inputs("s", Delivery::Steer).await.unwrap();
    assert_eq!(pending.len(), 1, "re-promoted row is claimable again");
}

#[tokio::test]
async fn recover_orphan_inputs_recovers_only_unrecorded_promoted_rows() {
    let store = mem().await;
    store.create_session(&meta_for("s", 1)).await.unwrap();
    let seq_a = store.admit_input(&steer("s", "a")).await.unwrap();
    let seq_b = store.admit_input(&steer("s", "b")).await.unwrap();
    let seq_c = store.admit_input(&steer("s", "c")).await.unwrap();
    // a: promoted + recorded (consumed). b: promoted, never recorded (the
    // crash orphan). c: never promoted.
    store.promote_inputs("s", 1, Delivery::Steer).await.unwrap();
    store.mark_inputs_recorded("s", &[seq_a]).await.unwrap();
    store.promote_inputs("s", 2, Delivery::Steer).await.unwrap();

    assert_eq!(
        store
            .pending_inputs("s", Delivery::Steer)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store.recover_orphan_inputs("s").await.unwrap(),
        1,
        "exactly the orphan recovers"
    );

    let pending = store.pending_inputs("s", Delivery::Steer).await.unwrap();
    assert_eq!(pending.len(), 2, "orphan b rejoins c");
    assert!(pending.iter().any(|i| i.seq == Some(seq_b)));
    assert!(pending.iter().any(|i| i.seq == Some(seq_c)));
    // Idempotent; unknown session is a clean zero.
    assert_eq!(store.recover_orphan_inputs("s").await.unwrap(), 0);
    assert_eq!(store.recover_orphan_inputs("nope").await.unwrap(), 0);
}

#[tokio::test]
async fn plain_admit_duplicate_rejects_with_the_unique_constraint_text() {
    let store = mem().await;
    store.create_session(&meta_for("s", 1)).await.unwrap();
    store.admit_input(&steer("s", "in-1")).await.unwrap();
    let err = store.admit_input(&steer("s", "in-1")).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("UNIQUE constraint failed: session_inputs.session_id, session_inputs.id"),
        "{err}"
    );
    // The same id in another session is a different row (no conflict).
    store.create_session(&meta_for("s2", 1)).await.unwrap();
    assert!(store.admit_input(&steer("s2", "in-1")).await.unwrap() > 0);
}

#[tokio::test]
async fn admit_into_missing_session_replicates_the_foreign_key_failure() {
    let store = mem().await;
    // No session row: SQL `session_inputs.session_id REFERENCES sessions(id)`
    // rejects the insert; the rdb backend replicates the failure text.
    let err = store
        .admit_input(&steer("missing", "fk-1"))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("FOREIGN KEY constraint failed"),
        "{err}"
    );
    // The idempotent lane fails the same way on its insert path (a plain
    // FK failure, not a typed InputConflict) and leaves nothing behind.
    let err = store
        .admit_input_once(&steer("missing", "fk-1"))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("FOREIGN KEY constraint failed"),
        "{err}"
    );
    assert!(err.downcast_ref::<InputConflict>().is_none(), "{err:#}");
    assert!(store
        .pending_inputs("missing", Delivery::Steer)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn admit_once_is_idempotent_and_rejects_different_payloads() {
    let store = mem().await;
    store.create_session(&meta_for("s", 1)).await.unwrap();
    let mut input = steer("s", "exec-1");
    input.images = vec!["https://example.test/image.png".into()];
    input.display_text = Some("display prompt".into());

    let first = store.admit_input_once(&input).await.unwrap();
    assert!(first.inserted);
    let retry = store.admit_input_once(&input).await.unwrap();
    assert_eq!(retry.seq, first.seq);
    assert!(!retry.inserted, "identical retry must not double-record");

    // Every semantic field change is a typed conflict.
    let mut conflicts = vec![{
        let mut i = input.clone();
        i.prompt = "different".into();
        i
    }];
    conflicts.push({
        let mut i = input.clone();
        i.delivery = Delivery::Queue;
        i
    });
    conflicts.push({
        let mut i = input.clone();
        i.images = vec!["https://example.test/other.png".into()];
        i
    });
    conflicts.push({
        let mut i = input.clone();
        i.display_text = None;
        i
    });
    for changed in conflicts {
        let err = store.admit_input_once(&changed).await.unwrap_err();
        assert!(err.downcast_ref::<InputConflict>().is_some(), "{err:#}");
    }
    // Nothing double-recorded: exactly one pending row for the session.
    assert_eq!(
        store
            .pending_inputs("s", Delivery::Steer)
            .await
            .unwrap()
            .len(),
        1
    );

    // Consumption state is NOT part of the payload: a retry after promote
    // and record still returns the original row.
    store.promote_inputs("s", 1, Delivery::Steer).await.unwrap();
    store.mark_inputs_recorded("s", &[first.seq]).await.unwrap();
    let consumed = store.admit_input_once(&input).await.unwrap();
    assert_eq!(consumed.seq, first.seq);
    assert!(!consumed.inserted);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_admit_once_with_different_payloads_keeps_exactly_one_row() {
    let store = std::sync::Arc::new(mem().await);
    store.create_session(&meta_for("s", 1)).await.unwrap();
    let mk = |prompt: &str| {
        let mut i = queue_input("s", "race-1");
        i.prompt = prompt.into();
        i
    };
    let first = mk("first contender");
    let second = mk("second contender");
    let (sa, sb) = (store.clone(), store.clone());
    let (a, b) = tokio::join!(sa.admit_input_once(&first), sb.admit_input_once(&second));
    let mut inserted = 0;
    let mut conflicts = 0;
    for result in [a, b] {
        match result {
            Ok(admitted) => {
                assert!(admitted.inserted);
                inserted += 1;
            }
            Err(err) => {
                assert!(err.downcast_ref::<InputConflict>().is_some(), "{err:#}");
                conflicts += 1;
            }
        }
    }
    assert_eq!(
        (inserted, conflicts),
        (1, 1),
        "one winner, one typed conflict"
    );
    assert_eq!(
        store
            .pending_inputs("s", Delivery::Queue)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn claim_next_queue_returns_oldest_with_clean_prompt_and_images() {
    let store = mem().await;
    store.create_session(&meta_for("s", 1)).await.unwrap();
    store.admit_input(&queue_input("s", "q-1")).await.unwrap();
    store.admit_input(&queue_input("s", "q-2")).await.unwrap();
    // A steer is not claimable by the queue drain.
    store.admit_input(&steer("s", "st-1")).await.unwrap();

    let (seq, claimed) = store
        .claim_next_queue("s")
        .await
        .unwrap()
        .expect("claim q-1");
    assert_eq!(
        claimed.prompt, "prompt-q-1",
        "LLM sees only the clean prompt"
    );
    assert_eq!(claimed.id, "q-1");
    assert!(seq > 0);

    // display_text rides along verbatim for the UI mirror.
    let store2 = mem().await;
    store2.create_session(&meta_for("s", 1)).await.unwrap();
    let mut input = queue_input("s", "q-display");
    input.display_text = Some("clean text $skill token".into());
    store2.admit_input(&input).await.unwrap();
    let (_, claimed) = store2.claim_next_queue("s").await.unwrap().unwrap();
    assert_eq!(
        claimed.display_text.as_deref(),
        Some("clean text $skill token")
    );

    // Images round-trip through the claim.
    let store3 = mem().await;
    store3.create_session(&meta_for("s", 1)).await.unwrap();
    let mut input = queue_input("s", "q-img");
    input.images = vec!["data:image/png;base64,YQ==".into()];
    store3.admit_input(&input).await.unwrap();
    let (_, claimed) = store3.claim_next_queue("s").await.unwrap().unwrap();
    assert_eq!(claimed.images, vec!["data:image/png;base64,YQ=="]);

    // Second claim on the same session: the promoted-but-unrecorded row is
    // held active, so the drain sees the NEXT queued item instead.
    let (seq2, claimed2) = store
        .claim_next_queue("s")
        .await
        .unwrap()
        .expect("claim q-2");
    assert_eq!(claimed2.id, "q-2");
    assert_ne!(seq, seq2);
    // Queue drained (steer excluded): no more claims.
    assert!(store.claim_next_queue("s").await.unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_claims_never_hand_out_the_same_row_twice() {
    let store = std::sync::Arc::new(mem().await);
    store.create_session(&meta_for("s", 1)).await.unwrap();
    store.admit_input(&queue_input("s", "only")).await.unwrap();

    // Two concurrent drains, one queued item: exactly one Some, one None.
    let (sa, sb) = (store.clone(), store.clone());
    let (a, b) = tokio::join!(sa.claim_next_queue("s"), sb.claim_next_queue("s"));
    let claims = [a.unwrap(), b.unwrap()];
    assert_eq!(
        claims.iter().filter(|c| c.is_some()).count(),
        1,
        "single-active guard"
    );
    assert!(claims.iter().any(|c| c.is_none()));

    // Two items, two concurrent drains: distinct rows, both consumed once.
    let store2 = std::sync::Arc::new(mem().await);
    store2.create_session(&meta_for("s", 1)).await.unwrap();
    store2.admit_input(&queue_input("s", "a")).await.unwrap();
    store2.admit_input(&queue_input("s", "b")).await.unwrap();
    let (sa, sb) = (store2.clone(), store2.clone());
    let (a, b) = tokio::join!(sa.claim_next_queue("s"), sb.claim_next_queue("s"));
    let claimed_ids: Vec<String> = [a.unwrap(), b.unwrap()]
        .into_iter()
        .flatten()
        .map(|(_, i)| i.id)
        .collect();
    assert_eq!(claimed_ids.len(), 2);
    assert_ne!(claimed_ids[0], claimed_ids[1]);
}

#[tokio::test]
async fn promote_next_queued_promotes_and_delete_and_swap_reorder_the_queue() {
    let store = mem().await;
    store.create_session(&meta_for("s", 1)).await.unwrap();
    let s1 = store.admit_input(&queue_input("s", "q-1")).await.unwrap();
    let s2 = store.admit_input(&queue_input("s", "q-2")).await.unwrap();
    let s3 = store.admit_input(&queue_input("s", "q-3")).await.unwrap();

    // promote_next_queued promotes exactly the oldest queued row.
    assert_eq!(store.promote_next_queued("s").await.unwrap(), Some(s1));
    // The promoted row is no longer pending.
    let pending = store.pending_inputs("s", Delivery::Queue).await.unwrap();
    assert_eq!(
        pending.iter().map(|i| i.id.clone()).collect::<Vec<_>>(),
        vec!["q-2", "q-3"]
    );
    assert_eq!(store.promote_next_queued("nope").await.unwrap(), None);

    // Swap reorders the drain order of the two pending rows.
    store.swap_input_order("s", s2, s3).await.unwrap();
    let pending = store.pending_inputs("s", Delivery::Queue).await.unwrap();
    assert_eq!(
        pending.iter().map(|i| i.id.clone()).collect::<Vec<_>>(),
        vec!["q-3", "q-2"],
        "swapped admitted_seq flips FIFO order"
    );
    // Swapping a promoted row is rejected.
    assert!(store.swap_input_order("s", s1, s2).await.is_err());

    // delete_input removes a pending row; the seq is addressable globally.
    store.delete_input(s2).await.unwrap();
    let pending = store.pending_inputs("s", Delivery::Queue).await.unwrap();
    assert_eq!(
        pending.iter().map(|i| i.id.clone()).collect::<Vec<_>>(),
        vec!["q-3"]
    );
    // Deleting an already-deleted seq is idempotent; unknown seqs are fine.
    store.delete_input(s2).await.unwrap();
    store.delete_input(999_999).await.unwrap();
}

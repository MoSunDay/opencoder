//! Inputs / follow-up-queue integration tests for the libsql-backed Store.
//!
//! Covers admit / promote / claim / swap / delete semantics for steer & queue
//! delivery. Split out of `store_integration.rs` to keep each file focused and
//! under the line-count limit. Runs against a real on-disk libsql file (tempdir).

use opencoder_store::{Delivery, LibsqlStore, SessionInput, SessionMeta, Store};
use tempfile::TempDir;

async fn fresh() -> (TempDir, LibsqlStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = LibsqlStore::open(dir.path().join("test.db")).await.unwrap();
    (dir, store)
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
    };
    store.create_session(&meta).await.unwrap();
}

#[tokio::test]
async fn inputs_steer_and_queue_promotion_semantics() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s", 1).await;

    let admit = |seq: i64, delivery: Delivery, prompt: &str| -> SessionInput {
        SessionInput {
            seq: None,
            id: format!("in-{seq}"),
            session_id: "s".into(),
            delivery,
            prompt: prompt.into(),
            images: Vec::new(),
            admitted_seq: seq,
            promoted_seq: None,
        }
    };

    store
        .admit_input(&admit(1, Delivery::Steer, "steer-1"))
        .await
        .unwrap();
    store
        .admit_input(&admit(2, Delivery::Queue, "queue-1"))
        .await
        .unwrap();
    store
        .admit_input(&admit(3, Delivery::Queue, "queue-2"))
        .await
        .unwrap();

    // pending: 1 steer + 2 queue
    let pending_steer = store.pending_inputs("s", Delivery::Steer).await.unwrap();
    assert_eq!(pending_steer.len(), 1);
    let pending_queue = store.pending_inputs("s", Delivery::Queue).await.unwrap();
    assert_eq!(pending_queue.len(), 2);

    // promote steers up to seq 1 → exactly the 1 steer promoted
    let promoted = store.promote_inputs("s", 1, Delivery::Steer).await.unwrap();
    assert_eq!(promoted.len(), 1);
    assert!(store
        .pending_inputs("s", Delivery::Steer)
        .await
        .unwrap()
        .is_empty());

    // promote_next_queued promotes exactly ONE (oldest), leaving the other pending
    let one = store.promote_next_queued("s").await.unwrap();
    assert_eq!(one, Some(2)); // admitted_seq ordering; seq of queue-1
    let still_pending = store.pending_inputs("s", Delivery::Queue).await.unwrap();
    assert_eq!(still_pending.len(), 1, "exactly one queue remains");
    let next = store.promote_next_queued("s").await.unwrap();
    assert!(next.is_some());
    assert!(store
        .pending_inputs("s", Delivery::Queue)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn swap_input_order_changes_drain_order() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s", 1).await;

    let admit = |seq: i64, delivery: Delivery, prompt: &str| -> SessionInput {
        SessionInput {
            seq: None,
            id: format!("in-{seq}"),
            session_id: "s".into(),
            delivery,
            prompt: prompt.into(),
            images: Vec::new(),
            admitted_seq: seq,
            promoted_seq: None,
        }
    };

    let seq_a = store
        .admit_input(&admit(1, Delivery::Queue, "A"))
        .await
        .unwrap();
    let _seq_b = store
        .admit_input(&admit(2, Delivery::Queue, "B"))
        .await
        .unwrap();
    let seq_c = store
        .admit_input(&admit(3, Delivery::Queue, "C"))
        .await
        .unwrap();

    let prompts: Vec<String> = store
        .pending_inputs("s", Delivery::Queue)
        .await
        .unwrap()
        .iter()
        .map(|i| i.prompt.clone())
        .collect();
    assert_eq!(
        prompts,
        vec!["A".to_string(), "B".to_string(), "C".to_string()]
    );

    store.swap_input_order("s", seq_a, seq_c).await.unwrap();

    let prompts: Vec<String> = store
        .pending_inputs("s", Delivery::Queue)
        .await
        .unwrap()
        .iter()
        .map(|i| i.prompt.clone())
        .collect();
    assert_eq!(
        prompts,
        vec!["C".to_string(), "B".to_string(), "A".to_string()]
    );

    // swapping against a non-existent seq must error
    assert!(store.swap_input_order("s", seq_a, 999999).await.is_err());
}

#[tokio::test]
async fn delete_input_removes_pending_and_preserves_order() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s", 1).await;

    let admit = |seq: i64, delivery: Delivery, prompt: &str| -> SessionInput {
        SessionInput {
            seq: None,
            id: format!("in-{seq}"),
            session_id: "s".into(),
            delivery,
            prompt: prompt.into(),
            images: Vec::new(),
            admitted_seq: seq,
            promoted_seq: None,
        }
    };

    let _seq_a = store
        .admit_input(&admit(1, Delivery::Queue, "A"))
        .await
        .unwrap();
    let seq_b = store
        .admit_input(&admit(2, Delivery::Queue, "B"))
        .await
        .unwrap();
    let _seq_c = store
        .admit_input(&admit(3, Delivery::Queue, "C"))
        .await
        .unwrap();

    // Delete the middle item; A and C remain in admitted_seq order.
    store.delete_input(seq_b).await.unwrap();

    let remaining: Vec<String> = store
        .pending_inputs("s", Delivery::Queue)
        .await
        .unwrap()
        .iter()
        .map(|i| i.prompt.clone())
        .collect();
    assert_eq!(remaining, vec!["A".to_string(), "C".to_string()]);

    // Idempotent: re-deleting matches 0 rows but is not an error.
    store.delete_input(seq_b).await.unwrap();
    assert_eq!(
        store
            .pending_inputs("s", Delivery::Queue)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn delete_input_preserves_already_promoted_audit_row() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s", 1).await;

    let admit = |seq: i64, delivery: Delivery, prompt: &str| -> SessionInput {
        SessionInput {
            seq: None,
            id: format!("in-{seq}"),
            session_id: "s".into(),
            delivery,
            prompt: prompt.into(),
            images: Vec::new(),
            admitted_seq: seq,
            promoted_seq: None,
        }
    };

    let seq_a = store
        .admit_input(&admit(1, Delivery::Queue, "A"))
        .await
        .unwrap();
    // Drain A (promoted_seq now set); nothing pending.
    assert!(store.promote_next_queued("s").await.unwrap().is_some());
    assert!(store
        .pending_inputs("s", Delivery::Queue)
        .await
        .unwrap()
        .is_empty());

    // Delete the already-promoted A — the `promoted_seq IS NULL` guard must
    // skip it, preserving the audit row.
    store.delete_input(seq_a).await.unwrap();

    // Re-admit C; its admitted_seq reveals whether A's row survived:
    //   guard present -> MAX(admitted_seq)=1 (A kept) -> C admitted_seq = 2
    //   guard absent  -> A deleted                  -> C admitted_seq = 1
    let _ = store
        .admit_input(&admit(2, Delivery::Queue, "C"))
        .await
        .unwrap();
    let pending = store.pending_inputs("s", Delivery::Queue).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].prompt, "C");
    assert_eq!(
        pending[0].admitted_seq, 2,
        "promoted audit row must be preserved"
    );
}

#[tokio::test]
async fn claim_next_queue_returns_seq_marks_promoted_and_idempotent_delete() {
    let (_dir, store) = fresh().await;
    make_session(&store, "s", 1).await;

    let admit = |seq: i64, prompt: &str| -> SessionInput {
        SessionInput {
            seq: None,
            id: format!("in-{seq}"),
            session_id: "s".into(),
            delivery: Delivery::Queue,
            prompt: prompt.into(),
            images: Vec::new(),
            admitted_seq: seq,
            promoted_seq: None,
        }
    };

    store.admit_input(&admit(1, "A")).await.unwrap();
    store.admit_input(&admit(2, "B")).await.unwrap();

    // Claim the oldest: returns its row seq + prompt, and marks it promoted.
    let (seq_a, input_a) = store
        .claim_next_queue("s")
        .await
        .unwrap()
        .expect("first claim");
    assert_eq!(input_a.prompt, "A");

    // Claim the next: second item, distinct seq.
    let (seq_b, input_b) = store
        .claim_next_queue("s")
        .await
        .unwrap()
        .expect("second claim");
    assert_eq!(input_b.prompt, "B");
    assert_ne!(seq_a, seq_b);

    // Nothing left to claim.
    assert!(store.claim_next_queue("s").await.unwrap().is_none());

    // A is already promoted: delete is a guarded no-op (audit row preserved).
    store.delete_input(seq_a).await.unwrap();
    assert!(store
        .pending_inputs("s", Delivery::Queue)
        .await
        .unwrap()
        .is_empty());
}

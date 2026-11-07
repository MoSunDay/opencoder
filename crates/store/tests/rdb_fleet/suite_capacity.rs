//! Host admission capacity: FIFO tickets, one live slot per execution,
//! completion and restart recovery.
use opencoder_store::fleet::FleetStore;

async fn configured(max_runs: usize) -> FleetStore {
    let store = FleetStore::open_memory().await.unwrap();
    store.configure_capacity(max_runs).await.unwrap();
    store
}

#[tokio::test]
async fn capacity_is_unset_until_initialized_and_configure_overrides() {
    let store = FleetStore::open_memory().await.unwrap();
    assert!(store.capacity().await.is_err(), "capacity starts unset");

    store.initialize_capacity(2).await.unwrap();
    let snapshot = store.capacity().await.unwrap();
    assert_eq!(
        (snapshot.max_runs, snapshot.running, snapshot.queued),
        (2, 0, 0)
    );

    store.initialize_capacity(5).await.unwrap();
    assert_eq!(
        store.capacity().await.unwrap().max_runs,
        2,
        "initialization never overwrites a durable value"
    );
    store.configure_capacity(3).await.unwrap();
    assert_eq!(store.capacity().await.unwrap().max_runs, 3);

    assert!(store.initialize_capacity(0).await.is_err());
    assert!(store.configure_capacity(0).await.is_err());
    assert_eq!(
        store.capacity().await.unwrap().max_runs,
        3,
        "invalid configuration leaves the stored value unchanged"
    );
}

#[tokio::test]
async fn enqueue_is_idempotent_and_allows_one_live_ticket_per_execution() {
    let store = configured(1).await;
    let first = store.enqueue_capacity("t1", "exec-1", "r1").await.unwrap();
    let replay = store.enqueue_capacity("t1", "exec-1", "r1").await.unwrap();
    assert_eq!(
        first, replay,
        "scheduler replays return the original sequence"
    );

    assert!(
        store.enqueue_capacity("t1", "exec-2", "r1").await.is_err(),
        "a ticket cannot change its execution"
    );
    let clash = store
        .enqueue_capacity("t2", "exec-1", "r1")
        .await
        .unwrap_err();
    assert!(
        clash.to_string().contains("UNIQUE constraint failed"),
        "{clash}"
    );
    let snapshot = store.capacity().await.unwrap();
    assert_eq!((snapshot.running, snapshot.queued), (0, 1));

    assert!(store.claim_capacity("t1", "r1").await.unwrap());
    assert!(
        !store.claim_capacity("t1", "r1").await.unwrap(),
        "already running"
    );
    assert!(!store.claim_capacity("missing", "r1").await.unwrap());
    assert!(
        !store.claim_capacity("t1", "r2").await.unwrap(),
        "wrong runtime"
    );
    let snapshot = store.capacity().await.unwrap();
    assert_eq!((snapshot.running, snapshot.queued), (1, 0));

    assert!(store.finish_capacity("t1", "r2").await.is_err());
    assert_eq!(store.capacity().await.unwrap().running, 1);
    store.finish_capacity("t1", "r1").await.unwrap();
    assert_eq!(store.capacity().await.unwrap().running, 0);
    assert!(store.runtime_tickets("r1").await.unwrap().is_empty());
    store.finish_capacity("t1", "r1").await.unwrap();
    assert_eq!(
        store.capacity().await.unwrap().running,
        0,
        "finish is idempotent"
    );

    store.enqueue_capacity("t2", "exec-1", "r1").await.unwrap();
    assert!(
        store.enqueue_capacity("t1", "exec-1", "r1").await.is_err(),
        "a completed ticket cannot be revived"
    );
}

#[tokio::test]
async fn claims_walk_the_global_fifo_head_only_while_a_slot_is_free() {
    let store = configured(1).await;
    for (ticket, execution) in [("t1", "exec-1"), ("t2", "exec-2"), ("t3", "exec-3")] {
        store
            .enqueue_capacity(ticket, execution, "r1")
            .await
            .unwrap();
    }
    assert!(
        !store.claim_capacity("t2", "r1").await.unwrap(),
        "not the head"
    );
    assert!(store.claim_capacity("t1", "r1").await.unwrap());
    assert!(
        !store.claim_capacity("t2", "r1").await.unwrap(),
        "the single slot is occupied"
    );
    assert!(
        !store.claim_capacity("t3", "r1").await.unwrap(),
        "not the head"
    );
    store.finish_capacity("t1", "r1").await.unwrap();
    assert!(
        !store.claim_capacity("t3", "r1").await.unwrap(),
        "still not the head"
    );
    assert!(store.claim_capacity("t2", "r1").await.unwrap());
    let snapshot = store.capacity().await.unwrap();
    assert_eq!((snapshot.running, snapshot.queued), (1, 1));
}

#[tokio::test]
async fn concurrent_claims_of_one_ticket_have_exactly_one_winner() {
    let store = configured(2).await;
    store
        .enqueue_capacity("ticket", "exec-1", "r1")
        .await
        .unwrap();
    let (left, right) = tokio::join!(
        store.claim_capacity("ticket", "r1"),
        store.claim_capacity("ticket", "r1")
    );
    let wins = [left.unwrap(), right.unwrap()]
        .into_iter()
        .filter(|won| *won)
        .count();
    assert_eq!(wins, 1);
    assert_eq!(store.capacity().await.unwrap().running, 1);
}

#[tokio::test]
async fn concurrent_enqueues_for_one_execution_claim_one_live_slot() {
    let store = configured(2).await;
    let (left, right) = tokio::join!(
        store.enqueue_capacity("ticket-a", "exec-1", "r1"),
        store.enqueue_capacity("ticket-b", "exec-1", "r2")
    );
    assert_eq!(
        usize::from(left.is_ok()) + usize::from(right.is_ok()),
        1,
        "one_live_slot survives the race"
    );
    assert_eq!(store.capacity().await.unwrap().queued, 1);
    let runtime = if left.is_ok() { "r1" } else { "r2" };
    assert_eq!(store.runtime_tickets(runtime).await.unwrap().len(), 1);
    let unknown = if left.is_ok() { "r2" } else { "r1" };
    assert!(store.runtime_tickets(unknown).await.unwrap().is_empty());
}

#[tokio::test]
async fn runtime_tickets_are_ordered_by_sequence_and_scoped_to_the_runtime() {
    let store = configured(2).await;
    for (ticket, execution, runtime) in [
        ("t1", "exec-1", "r1"),
        ("t2", "exec-2", "r1"),
        ("t3", "exec-3", "r2"),
    ] {
        store
            .enqueue_capacity(ticket, execution, runtime)
            .await
            .unwrap();
    }
    assert_eq!(
        store.runtime_tickets("r1").await.unwrap(),
        vec![
            ("t1".to_owned(), "exec-1".to_owned(), "queued".to_owned()),
            ("t2".to_owned(), "exec-2".to_owned(), "queued".to_owned()),
        ]
    );
    assert!(store.claim_capacity("t1", "r1").await.unwrap());
    assert!(store.claim_capacity("t2", "r1").await.unwrap());
    let phases: Vec<_> = store
        .runtime_tickets("r1")
        .await
        .unwrap()
        .into_iter()
        .map(|(ticket, _, phase)| (ticket, phase))
        .collect();
    assert_eq!(
        phases,
        vec![
            ("t1".to_owned(), "running".to_owned()),
            ("t2".to_owned(), "running".to_owned()),
        ]
    );
    store.finish_capacity("t2", "r1").await.unwrap();
    assert_eq!(
        store.runtime_tickets("r1").await.unwrap(),
        vec![("t1".to_owned(), "exec-1".to_owned(), "running".to_owned())]
    );
    assert_eq!(
        store.runtime_tickets("r2").await.unwrap(),
        vec![("t3".to_owned(), "exec-3".to_owned(), "queued".to_owned())]
    );
    assert!(
        store.claim_capacity("t3", "r2").await.unwrap(),
        "finishing a ticket frees a global slot for the next queued runtime"
    );
    assert_eq!(store.capacity().await.unwrap().running, 2);
}

#[tokio::test]
async fn recover_releases_only_running_tickets_with_matching_identity() {
    let store = configured(1).await;
    store.enqueue_capacity("t1", "exec-1", "r1").await.unwrap();
    assert!(store.claim_capacity("t1", "r1").await.unwrap());

    assert!(store.recover_capacity("t1", "exec-2", "r1").await.is_err());
    assert!(store.recover_capacity("t1", "exec-1", "r2").await.is_err());
    assert_eq!(store.capacity().await.unwrap().running, 1);

    assert!(store.recover_capacity("t1", "exec-1", "r1").await.unwrap());
    assert_eq!(store.capacity().await.unwrap().running, 0);
    assert!(store.runtime_tickets("r1").await.unwrap().is_empty());
    assert!(
        !store.recover_capacity("t1", "exec-1", "r1").await.unwrap(),
        "recovering a completed ticket is a no-op"
    );
    assert!(
        store.enqueue_capacity("t2", "exec-1", "r1").await.is_ok(),
        "recovery releases the live slot"
    );

    store.enqueue_capacity("t3", "exec-3", "r1").await.unwrap();
    assert!(
        store.recover_capacity("t2", "exec-1", "r1").await.is_err(),
        "a queued ticket is not running"
    );
    assert!(store
        .recover_capacity("missing", "exec-1", "r1")
        .await
        .is_err());
}

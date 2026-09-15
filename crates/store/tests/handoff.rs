use opencoder_core::fleet::*;
use opencoder_store::fleet::{handoff::RuntimeRecord, FleetStore};
use serde_json::json;

fn assignment() -> Assignment {
    Assignment {
        runtime: None,
        codex: None,
        index: ExecutionIndex {
            id: "agent-once".into(),
            node_id: "node-one".into(),
            kind: ExecutionKind::Agent,
            created_at: 17,
            status: ExecutionStatus::Pending,
        },
        request: CreateExecution {
            id: "agent-once".into(),
            kind: ExecutionKind::Agent,
            target: Some("act".into()),
            input: json!({"prompt":"once"}),
            node_id: None,
        },
        definition: Some(json!({"version":1})),
    }
}

#[tokio::test]
async fn dispatch_survives_reopen_and_preserves_original_assignment() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.db");
    let first = FleetStore::open(&path).await.unwrap();
    let second = FleetStore::open(&path).await.unwrap();
    let a = assignment();
    assert!(first
        .claim_request("execution", &a.index.id, "original")
        .await
        .unwrap());
    first.prepare_assignment(&a, "original").await.unwrap();
    assert!(!second
        .claim_request("execution", &a.index.id, "changed")
        .await
        .unwrap());
    assert_eq!(
        second
            .assignment(&a.index.id)
            .await
            .unwrap()
            .unwrap()
            .definition,
        a.definition
    );
    // A candidate's initial inventory can race an in-flight outbox dispatch.
    second
        .apply_index_report("node-one", &[], Some(std::slice::from_ref(&a.index.id)))
        .await
        .unwrap();
    assert_eq!(
        second.index(&a.index.id).await.unwrap().unwrap().status,
        ExecutionStatus::Pending
    );
    let reply = RpcReply {
        status: 202,
        body: json!(a.index),
    };
    first
        .finish_dispatch(&a.index.id, "original", &reply)
        .await
        .unwrap();
    drop(first);
    drop(second);
    let reopened = FleetStore::open(&path).await.unwrap();
    let receipt = reopened
        .receipt("execution", &a.index.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.phase, "accepted");
    assert_eq!(receipt.payload["body"]["created_at"], 17);
    assert_eq!(
        reopened
            .assignment(&a.index.id)
            .await
            .unwrap()
            .unwrap()
            .request,
        a.request
    );
}

#[tokio::test]
async fn process_lock_serializes_independent_connections_and_releases_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.db");
    let first = FleetStore::open(&path).await.unwrap();
    let second = std::sync::Arc::new(FleetStore::open(&path).await.unwrap());
    let lock = first.request_lock("brain", "request").await.unwrap();
    let other = second.clone();
    let waiting =
        tokio::spawn(async move { other.request_lock("brain", "request").await.unwrap() });
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(!waiting.is_finished());
    // Unrelated requests continue while a model operation holds this key.
    drop(second.request_lock("brain", "another").await.unwrap());
    drop(lock);
    drop(
        tokio::time::timeout(std::time::Duration::from_secs(2), waiting)
            .await
            .unwrap()
            .unwrap(),
    );
}

#[tokio::test]
async fn ownership_and_fifo_capacity_span_three_releases_and_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let store = FleetStore::open(&path).await.unwrap();
    store.configure_capacity(1).await.unwrap();
    for id in ["r1", "r2", "r3"] {
        store
            .register_runtime(&RuntimeRecord {
                id: id.into(),
                release_id: format!("release-{id}"),
                config: json!({}),
                mode: "staged".into(),
            })
            .await
            .unwrap();
    }
    store.activate_runtime("r1").await.unwrap();
    assert_eq!(
        store.assign_runtime("agent-long", None).await.unwrap(),
        "r1"
    );
    store
        .enqueue_capacity("t1", "agent-long", "r1")
        .await
        .unwrap();
    assert!(store.claim_capacity("t1", "r1").await.unwrap());
    store.activate_runtime("r2").await.unwrap();
    assert_eq!(
        store.assign_runtime("agent-next", None).await.unwrap(),
        "r2"
    );
    store
        .enqueue_capacity("t2", "agent-next", "r2")
        .await
        .unwrap();
    store.activate_runtime("r3").await.unwrap();
    assert_eq!(
        store.assign_runtime("agent-long", None).await.unwrap(),
        "r1"
    );
    assert_eq!(
        store
            .assign_runtime("agent-child", Some("r1"))
            .await
            .unwrap(),
        "r1"
    );
    assert!(store
        .assign_runtime("agent-long", Some("r3"))
        .await
        .is_err());
    store
        .enqueue_capacity("t3", "agent-new", "r3")
        .await
        .unwrap();
    assert!(!store.claim_capacity("t2", "r2").await.unwrap());
    drop(store);
    // A Host restart does not release another process's reservations.
    let store = FleetStore::open(&path).await.unwrap();
    assert_eq!(store.capacity().await.unwrap().running, 1);
    store.activate_runtime("r1").await.unwrap();
    assert_eq!(
        store.owner("agent-next").await.unwrap().unwrap().runtime_id,
        "r2"
    );
    store.finish_capacity("t1", "r1").await.unwrap();
    assert!(!store.claim_capacity("t3", "r3").await.unwrap());
    assert!(store.claim_capacity("t2", "r2").await.unwrap());
    assert!(!store.claim_capacity("t2", "r2").await.unwrap());
    assert_eq!(store.capacity().await.unwrap().running, 1);
    store.finish_capacity("t2", "r2").await.unwrap();
    assert!(store.claim_capacity("t3", "r3").await.unwrap());
}

#[tokio::test]
async fn stale_server_reports_cannot_overwrite_new_host_state() {
    let store = FleetStore::open_memory().await.unwrap();
    let mut index = assignment().index;
    index.status = ExecutionStatus::Running;
    store
        .apply_index_report_fenced(
            "node-one",
            &[index.clone()],
            None,
            Some(("host-0002-new", 1)),
        )
        .await
        .unwrap();
    index.status = ExecutionStatus::Pending;
    store
        .apply_index_report_fenced(
            "node-one",
            &[index.clone()],
            None,
            Some(("host-0001-old", 999)),
        )
        .await
        .unwrap();
    store
        .apply_index_report_fenced(
            "node-one",
            &[index.clone()],
            None,
            Some(("host-0002-new", 1)),
        )
        .await
        .unwrap();
    assert_eq!(
        store.index(&index.id).await.unwrap().unwrap().status,
        ExecutionStatus::Running
    );
    index.status = ExecutionStatus::Done;
    store
        .apply_index_report_fenced(
            "node-one",
            &[index.clone()],
            None,
            Some(("host-0002-new", 2)),
        )
        .await
        .unwrap();
    assert_eq!(
        store.index(&index.id).await.unwrap().unwrap().status,
        ExecutionStatus::Done
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn historical_owner_reads_do_not_wait_for_another_process_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let store = FleetStore::open(&path).await.unwrap();
    store
        .register_runtime(&RuntimeRecord {
            id: "r1".into(),
            release_id: "release-one".into(),
            config: json!({}),
            mode: "staged".into(),
        })
        .await
        .unwrap();
    store.activate_runtime("r1").await.unwrap();
    store.assign_runtime("agent-history", None).await.unwrap();
    let database = libsql::Builder::new_local(&path).build().await.unwrap();
    let connection = database.connect().unwrap();
    let writer = connection
        .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
        .await
        .unwrap();
    let mut read = tokio::spawn(async move {
        let owner = store
            .assign_runtime("agent-history", Some("r1"))
            .await
            .unwrap();
        let conflict = store
            .assign_runtime("agent-history", Some("r2"))
            .await
            .is_err();
        (owner, conflict)
    });
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), &mut read).await;
    writer.rollback().await.unwrap();
    let (owner, conflict) = result
        .expect("historical routing waited for the writer lock")
        .unwrap();
    assert_eq!(owner, "r1");
    assert!(
        conflict,
        "immutable ownership conflict must still be rejected"
    );
}

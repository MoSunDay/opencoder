//! The frozen-assignment half of the handoff flow: prepare idempotency and
//! conflict rollback, receipt-gated writes, pending replay paging and the
//! node-acceptance transition.
use crate::common::frozen;
use opencoder_core::fleet::{ExecutionIndex, ExecutionKind, ExecutionStatus, RpcReply};
use opencoder_store::fleet::FleetStore;
use serde_json::json;

async fn prepare_new(store: &FleetStore, task: &opencoder_core::fleet::Assignment) -> String {
    let fingerprint = format!("fp-{}", task.index.id);
    assert!(store
        .claim_request("execution", &task.index.id, &fingerprint)
        .await
        .unwrap());
    store.prepare_assignment(task, &fingerprint).await.unwrap();
    fingerprint
}

fn ids(assignments: Vec<opencoder_core::fleet::Assignment>) -> Vec<String> {
    assignments
        .into_iter()
        .map(|assignment| assignment.index.id)
        .collect()
}

#[tokio::test]
async fn repeated_prepare_preserves_the_frozen_assignment_and_rolls_back_conflicts() {
    for kind in [ExecutionKind::Agent, ExecutionKind::Dag] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.db");
        let mut original = frozen(&format!("{}-replay", kind.prefix()), kind, 17);
        original.index.id = format!("{}-replay", kind.prefix());
        original.request.id = original.index.id.clone();
        original.request.kind = kind;

        {
            let store = FleetStore::open(&path).await.unwrap();
            assert!(store
                .claim_request("execution", &original.index.id, "original")
                .await
                .unwrap());
            store
                .prepare_assignment(&original, "original")
                .await
                .unwrap();
        }

        for _ in 0..3 {
            let store = FleetStore::open(&path).await.unwrap();
            let mut replay = original.clone();
            replay.definition = Some(json!({"version": 2}));
            store.prepare_assignment(&replay, "original").await.unwrap();
            assert_eq!(
                serde_json::to_value(store.assignment(&original.index.id).await.unwrap().unwrap())
                    .unwrap(),
                serde_json::to_value(&original).unwrap(),
                "the frozen assignment never changes on replay"
            );
            assert_eq!(store.pending_assignments("", 128).await.unwrap().len(), 1);
            let receipt = store
                .receipt("execution", &original.index.id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(receipt.phase, "prepared");

            let mut moved = original.clone();
            moved.index.node_id = "node-other".into();
            assert!(store.prepare_assignment(&moved, "original").await.is_err());
            let mut changed = original.clone();
            changed.request.input = json!({"prompt": "changed"});
            assert!(store
                .prepare_assignment(&changed, "original")
                .await
                .is_err());
            assert_eq!(
                serde_json::to_value(store.assignment(&original.index.id).await.unwrap().unwrap())
                    .unwrap(),
                serde_json::to_value(&original).unwrap(),
                "failed replay writes nothing"
            );
            store
                .prepare_assignment(&original, "original")
                .await
                .unwrap();
        }

        let store = FleetStore::open(&path).await.unwrap();
        let mut accepted = original.index.clone();
        accepted.status = ExecutionStatus::Running;
        let reply = RpcReply {
            status: 202,
            body: json!(accepted),
        };
        store
            .finish_dispatch(&original.index.id, "original", &reply)
            .await
            .unwrap();
        assert!(store.pending_assignments("", 128).await.unwrap().is_empty());
        assert_eq!(
            store
                .receipt("execution", &original.index.id)
                .await
                .unwrap()
                .unwrap()
                .phase,
            "accepted"
        );
    }
}

#[tokio::test]
async fn prepare_without_a_matching_receipt_writes_nothing() {
    let store = FleetStore::open_memory().await.unwrap();
    let task = frozen("dag-unclaimed", ExecutionKind::Dag, 3);
    let error = store
        .prepare_assignment(&task, "missing")
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("missing dispatch receipt"),
        "{error}"
    );
    assert!(store.index("dag-unclaimed").await.unwrap().is_none());
    assert!(store.assignment("dag-unclaimed").await.unwrap().is_none());
    assert!(store
        .receipt("execution", "dag-unclaimed")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn pending_assignments_page_by_id_with_after_and_limit() {
    let store = FleetStore::open_memory().await.unwrap();
    for (id, created_at) in [("agent-a", 1), ("agent-b", 2), ("agent-c", 3)] {
        let task = frozen(id, ExecutionKind::Agent, created_at);
        prepare_new(&store, &task).await;
    }
    assert_eq!(
        ids(store.pending_assignments("", 2).await.unwrap()),
        ["agent-a", "agent-b"]
    );
    assert_eq!(
        ids(store.pending_assignments("agent-b", 2).await.unwrap()),
        ["agent-c"]
    );
    assert!(store
        .pending_assignments("agent-c", 2)
        .await
        .unwrap()
        .is_empty());
    assert!(
        store.pending_assignments("", 0).await.unwrap().is_empty(),
        "zero limit returns no rows"
    );
    assert_eq!(store.pending_assignments("", 128).await.unwrap().len(), 3);

    let accepted = store.assignment("agent-b").await.unwrap().unwrap().index;
    let reply = RpcReply {
        status: 202,
        body: json!(accepted),
    };
    store
        .finish_dispatch("agent-b", "fp-agent-b", &reply)
        .await
        .unwrap();
    assert_eq!(
        ids(store.pending_assignments("", 128).await.unwrap()),
        ["agent-a", "agent-c"]
    );
    assert_eq!(
        ids(store.pending_assignments("agent-a", 128).await.unwrap()),
        ["agent-c"]
    );
}

#[tokio::test]
async fn finish_dispatch_transitions_the_receipt_and_requires_acceptance() {
    let store = FleetStore::open_memory().await.unwrap();
    let task = frozen("agent-finish", ExecutionKind::Agent, 9);
    let fingerprint = prepare_new(&store, &task).await;
    assert_eq!(store.pending_assignments("", 10).await.unwrap().len(), 1);
    assert!(store.index("agent-finish").await.unwrap().is_none());

    let accepted = ExecutionIndex {
        status: ExecutionStatus::Running,
        ..task.index.clone()
    };
    let rejection = RpcReply::error(500, "node refused");
    let error = store
        .finish_dispatch("agent-finish", &fingerprint, &rejection)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("node acceptance required"),
        "{error}"
    );
    assert_eq!(
        store
            .receipt("execution", "agent-finish")
            .await
            .unwrap()
            .unwrap()
            .phase,
        "prepared"
    );

    let reply = RpcReply {
        status: 202,
        body: json!(accepted),
    };
    store
        .finish_dispatch("agent-finish", &fingerprint, &reply)
        .await
        .unwrap();
    let receipt = store
        .receipt("execution", "agent-finish")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.phase, "accepted");
    assert_eq!(receipt.payload["body"]["status"], "running");
    assert_eq!(
        store.index("agent-finish").await.unwrap().unwrap(),
        accepted
    );
    assert!(store.pending_assignments("", 10).await.unwrap().is_empty());

    let conflict = store
        .finish_dispatch("agent-finish", "other-fingerprint", &reply)
        .await
        .unwrap_err();
    assert!(
        conflict
            .to_string()
            .contains("request receipt ownership conflict"),
        "{conflict}"
    );
    store
        .finish_dispatch("agent-finish", &fingerprint, &reply)
        .await
        .unwrap();
    assert_eq!(
        store
            .receipt("execution", "agent-finish")
            .await
            .unwrap()
            .unwrap()
            .phase,
        "accepted",
        "acceptance replay is idempotent"
    );
}

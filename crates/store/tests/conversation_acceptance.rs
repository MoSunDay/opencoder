use opencoder_core::fleet::*;
use opencoder_store::fleet::FleetStore;
use serde_json::json;

fn assignment(kind: ExecutionKind) -> Assignment {
    let id = format!("{}-acceptance", kind.prefix());
    Assignment {
        private_context: None,
        runtime: None,
        codex: None,
        definition: None,
        index: ExecutionIndex {
            id: id.clone(),
            created_at: 1,
            kind,
            node_id: "owner".into(),
            status: ExecutionStatus::Pending,
        },
        request: CreateExecution {
            id,
            kind,
            target: Some("reviewer".into()),
            input: json!({"prompt":"do once"}),
            node_id: Some("owner".into()),
        },
    }
}

async fn prepared(store: &FleetStore, kind: ExecutionKind) -> Assignment {
    let task = assignment(kind);
    assert!(store
        .claim_request("execution", &task.index.id, "original")
        .await
        .unwrap());
    store.prepare_assignment(&task, "original").await.unwrap();
    task
}

#[tokio::test]
async fn conversations_publish_only_remote_acceptance_and_replay_does_not_regress_status() {
    for kind in [ExecutionKind::Agent, ExecutionKind::Operator] {
        let store = FleetStore::open_memory().await.unwrap();
        let task = prepared(&store, kind).await;
        assert!(store.index(&task.index.id).await.unwrap().is_none());
        assert!(store
            .indexes(None, Some(kind), 10)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(store.pending_assignments("", 10).await.unwrap().len(), 1);
        let reply = RpcReply {
            status: 202,
            body: json!(task.index),
        };
        store
            .finish_dispatch(&task.index.id, "original", &reply)
            .await
            .unwrap();
        assert_eq!(
            store.index(&task.index.id).await.unwrap(),
            Some(task.index.clone())
        );
        assert!(store.pending_assignments("", 10).await.unwrap().is_empty());
        let done = ExecutionIndex {
            status: ExecutionStatus::Idle,
            ..task.index.clone()
        };
        store
            .apply_index_report("owner", std::slice::from_ref(&done), None)
            .await
            .unwrap();
        store.accept_execution(&task.index).await.unwrap();
        store
            .finish_dispatch(&task.index.id, "original", &reply)
            .await
            .unwrap();
        assert_eq!(store.index(&task.index.id).await.unwrap(), Some(done));
        assert_eq!(store.indexes(None, Some(kind), 10).await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn invalid_receipt_cannot_publish_an_index_or_replace_its_owner() {
    let store = FleetStore::open_memory().await.unwrap();
    let task = prepared(&store, ExecutionKind::Operator).await;
    let reply = RpcReply {
        status: 202,
        body: json!(task.index),
    };
    assert!(store
        .finish_dispatch(&task.index.id, "different", &reply)
        .await
        .is_err());
    assert!(store.index(&task.index.id).await.unwrap().is_none());
    let other = ExecutionIndex {
        node_id: "other".into(),
        ..task.index.clone()
    };
    assert!(store.accept_execution(&other).await.is_err());
    assert!(store
        .apply_index_report("other", &[other], None)
        .await
        .is_err());
    assert!(store.index(&task.index.id).await.unwrap().is_none());
    let rejected = RpcReply::error(400, "resource missing");
    assert!(store
        .finish_dispatch(&task.index.id, "original", &rejected)
        .await
        .is_err());
    assert!(store.index(&task.index.id).await.unwrap().is_none());
}

#[tokio::test]
async fn a_reply_cannot_accept_a_different_prepared_request() {
    let store = FleetStore::open_memory().await.unwrap();
    let first = prepared(&store, ExecutionKind::Operator).await;
    let second = prepared(&store, ExecutionKind::Agent).await;
    let reply = RpcReply {
        status: 202,
        body: json!(second.index),
    };
    assert!(store
        .finish_dispatch(&first.index.id, "original", &reply)
        .await
        .is_err());
    for task in [&first, &second] {
        assert!(store.index(&task.index.id).await.unwrap().is_none());
        assert_eq!(
            store
                .receipt("execution", &task.index.id)
                .await
                .unwrap()
                .unwrap()
                .phase,
            "prepared"
        );
    }
}

#[tokio::test]
async fn reconnect_report_recovers_lost_acceptance_without_a_local_placeholder() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("control.db");
    let store = FleetStore::open(&path).await.unwrap();
    let task = prepared(&store, ExecutionKind::Agent).await;
    drop(store);
    let store = FleetStore::open(&path).await.unwrap();
    assert!(store.index(&task.index.id).await.unwrap().is_none());
    assert_eq!(
        store.pending_assignments("", 10).await.unwrap()[0].request,
        task.request
    );
    let running = ExecutionIndex {
        status: ExecutionStatus::Running,
        ..task.index.clone()
    };
    store
        .apply_index_report("owner", std::slice::from_ref(&running), None)
        .await
        .unwrap();
    store
        .finish_dispatch(
            &task.index.id,
            "original",
            &RpcReply {
                status: 202,
                body: json!(task.index),
            },
        )
        .await
        .unwrap();
    assert_eq!(store.index(&task.index.id).await.unwrap(), Some(running));
}

#[tokio::test]
async fn dag_dispatch_keeps_its_existing_pending_index() {
    let store = FleetStore::open_memory().await.unwrap();
    let task = prepared(&store, ExecutionKind::Dag).await;
    assert_eq!(store.index(&task.index.id).await.unwrap(), Some(task.index));
}

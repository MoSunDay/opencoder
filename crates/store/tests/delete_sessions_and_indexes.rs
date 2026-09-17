//! Store seams behind the compat DELETE /api/nodes/:id/dialogs relay:
//! - `delete_sessions` removes session rows (FK cascade takes messages) and
//!   reports how many rows it actually deleted, ignoring unknown ids.
//! - `delete_terminal_indexes` deletes only droppable index rows
//!   (idle|done|error|cancelled) owned by the node+kind, together with their
//!   frozen assignments and execution receipts; live rows are refused.

use opencoder_core::fleet::*;
use opencoder_core::{ContentBlock, Message, Role};
use opencoder_store::fleet::FleetStore;
use opencoder_store::{LibsqlStore, SessionFilter, SessionMeta, Store};

async fn fresh() -> LibsqlStore {
    LibsqlStore::open_memory().await.unwrap()
}

fn meta(id: &str) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        title: Some(id.into()),
        agent: Some("act".into()),
        ..Default::default()
    }
}

async fn listed(store: &LibsqlStore) -> Vec<String> {
    store
        .list_sessions(&SessionFilter {
            limit: 100,
            ..Default::default()
        })
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect()
}

#[tokio::test]
async fn delete_sessions_cascades_and_ignores_unknown_ids() {
    let store = fresh().await;
    for id in ["keep", "drop-a", "drop-b"] {
        store.create_session(&meta(id)).await.unwrap();
    }
    store
        .append_message(
            "drop-a",
            &Message {
                provider_state: None,
                display: None,
                id: "m1".into(),
                role: Role::User,
                blocks: vec![ContentBlock::text("cascade me")],
                model: None,
                agent: None,
                usage: Default::default(),
                created_at: 2,
                synthetic: false,
            },
        )
        .await
        .unwrap();

    let removed = store
        .delete_sessions(&["drop-a".into(), "drop-b".into(), "ghost".into()])
        .await
        .unwrap();
    assert_eq!(removed, 2);
    assert_eq!(listed(&store).await, vec!["keep".to_string()]);
    assert!(store.load_messages("drop-a").await.unwrap().is_empty());
}

#[tokio::test]
async fn delete_terminal_indexes_only_removes_droppable_rows() {
    let fleet = FleetStore::open_memory().await.unwrap();
    let statuses = [
        (ExecutionStatus::Idle, true),
        (ExecutionStatus::Done, true),
        (ExecutionStatus::Error, true),
        (ExecutionStatus::Cancelled, true),
        (ExecutionStatus::Pending, false),
        (ExecutionStatus::Running, false),
        (ExecutionStatus::Cancelling, false),
        (ExecutionStatus::Interrupted, false),
    ];
    for (index, (status, _)) in statuses.iter().enumerate() {
        let id = format!("operator-{index}");
        fleet
            .claim_request("execution", &id, "fp")
            .await
            .unwrap();
        let assignment = Assignment {
            runtime: None,
            codex: None,
            index: ExecutionIndex {
                id: id.clone(),
                created_at: 1,
                kind: ExecutionKind::Operator,
                node_id: "node-x".into(),
                status: *status,
            },
            request: CreateExecution {
                id: id.clone(),
                kind: ExecutionKind::Operator,
                target: None,
                input: serde_json::json!({"prompt": "hi"}),
                node_id: Some("node-x".into()),
            },
            definition: None,
        };
        fleet.prepare_assignment(&assignment, "fp").await.unwrap();
    }

    let ids: Vec<String> = (0..statuses.len()).map(|i| format!("operator-{i}")).collect();
    let removed = fleet
        .delete_terminal_indexes("node-x", ExecutionKind::Operator, &ids)
        .await
        .unwrap();
    assert_eq!(removed, 4);

    let rows = fleet
        .indexes(Some("node-x"), Some(ExecutionKind::Operator), 100)
        .await
        .unwrap();
    let mut kept: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    kept.sort();
    assert_eq!(
        kept,
        vec![
            "operator-4".to_string(),
            "operator-5".to_string(),
            "operator-6".to_string(),
            "operator-7".to_string(),
        ]
    );
    // Assignments and receipts for deleted rows go with them; live rows keep
    // theirs so an in-flight dispatch is untouched.
    for (index, (_, droppable)) in statuses.iter().enumerate() {
        let id = format!("operator-{index}");
        let receipt = fleet.receipt("execution", &id).await.unwrap();
        let assignment = fleet.assignment(&id).await.unwrap();
        if *droppable {
            assert!(receipt.is_none(), "{id} receipt survived");
            assert!(assignment.is_none(), "{id} assignment survived");
        } else {
            assert!(receipt.is_some(), "{id} receipt lost");
            assert!(assignment.is_some(), "{id} assignment lost");
        }
    }
}

#[tokio::test]
async fn delete_terminal_indexes_is_ownership_guarded() {
    let fleet = FleetStore::open_memory().await.unwrap();
    let assignment = Assignment {
        runtime: None,
        codex: None,
        index: ExecutionIndex {
            id: "operator-own".into(),
            created_at: 1,
            kind: ExecutionKind::Operator,
            node_id: "node-a".into(),
            status: ExecutionStatus::Idle,
        },
        request: CreateExecution {
            id: "operator-own".into(),
            kind: ExecutionKind::Operator,
            target: None,
            input: serde_json::json!({"prompt": "hi"}),
            node_id: Some("node-a".into()),
        },
        definition: None,
    };
    fleet
        .claim_request("execution", "operator-own", "fp")
        .await
        .unwrap();
    fleet.prepare_assignment(&assignment, "fp").await.unwrap();

    // Wrong node or wrong kind must not touch the row.
    assert_eq!(
        fleet
            .delete_terminal_indexes("node-b", ExecutionKind::Operator, &["operator-own".into()])
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        fleet
            .delete_terminal_indexes("node-a", ExecutionKind::Agent, &["operator-own".into()])
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        fleet
            .delete_terminal_indexes("node-a", ExecutionKind::Operator, &["operator-own".into()])
            .await
            .unwrap(),
        1
    );
}

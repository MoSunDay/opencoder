use super::*;
use crate::fleet::{FleetConnection, FleetStore};
use opencoder_core::fleet::{Assignment, ExecutionKind, ExecutionStatus};
use turso::params;

fn index(id: &str) -> ExecutionIndex {
    ExecutionIndex {
        id: id.into(),
        created_at: i64::MAX - 1,
        kind: ExecutionKind::Agent,
        node_id: "node-owner".into(),
        status: ExecutionStatus::Idle,
    }
}
fn assignment(id: &str) -> Assignment {
    serde_json::from_value(serde_json::json!({"index":index(id),
        "request":{"id":id,"kind":"agent","input":{}},
        "definition":{"payload":"x".repeat(4096)}}))
    .unwrap()
}

#[tokio::test]
async fn first_report_checks_frozen_owner_and_rolls_back_the_whole_batch() {
    let store = FleetStore::open_memory().await.unwrap();
    store
        .db
        .write(|conn| async move {
            conn.execute(
                "INSERT INTO execution_assignments VALUES(?1,?2)",
                params![
                    "agent-one",
                    serde_json::to_string(&assignment("agent-one"))?
                ],
            )
            .await?;
            let fleet = FleetConnection { conn };
            let mut conflict = index("agent-one");
            conflict.created_at -= 1;
            assert!(fleet
                .apply_index_report("node-owner", &[index("agent-new"), conflict], None)
                .await
                .is_err());
            assert!(fleet.index("agent-new").await?.is_none());
            fleet
                .apply_index_report("node-owner", &[index("agent-one")], None)
                .await?;
            assert_eq!(
                fleet.index("agent-one").await?.unwrap().created_at,
                i64::MAX - 1
            );
            // Published reports must not parse or copy frozen request bodies.
            fleet
                .conn
                .execute("UPDATE execution_assignments SET assignment='not-json'", ())
                .await?;
            fleet
                .apply_index_report("node-owner", &[index("agent-one")], None)
                .await?;
            let mut conflict = index("agent-one");
            conflict.node_id = "another-node".into();
            assert!(fleet
                .apply_index_report("another-node", &[conflict], None)
                .await
                .is_err());
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn native_preparation_preserves_indexed_owners_before_and_after_acceptance() {
    let store = FleetStore::open_memory().await.unwrap();
    let original = assignment("agent-owner");
    assert!(store
        .claim_request("execution", &original.index.id, "fingerprint")
        .await
        .unwrap());
    store
        .prepare_assignment(&original, "fingerprint")
        .await
        .unwrap();
    for accepted in [false, true] {
        if accepted {
            store
                .apply_index_report("node-owner", std::slice::from_ref(&original.index), None)
                .await
                .unwrap();
        }
        for field in ["node", "created", "kind"] {
            let mut replacement = original.clone();
            match field {
                "node" => replacement.index.node_id = "another-node".into(),
                "created" => replacement.index.created_at -= 1,
                _ => {
                    replacement.index.kind = ExecutionKind::Operator;
                    replacement.request.kind = ExecutionKind::Operator;
                }
            }
            assert!(store
                .prepare_assignment(&replacement, "fingerprint")
                .await
                .is_err());
        }
    }
}

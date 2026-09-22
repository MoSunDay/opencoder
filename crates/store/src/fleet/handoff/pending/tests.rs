use super::*;
use crate::fleet::handoff::{dispatch_key, Receipt};
use serde_json::json;

async fn prepare(
    store: &FleetStore,
    id: &str,
    kind: &str,
    run: Option<&str>,
    phase: &str,
) -> Assignment {
    let assignment: Assignment = serde_json::from_value(json!({
        "index": {"id": id, "created_at": 1, "kind": kind,
            "node_id": "node-one", "status": "pending"},
        "request": {"id": id, "kind": kind, "input": {"run_id": run}},
        "definition": {"name": id}
    }))
    .unwrap();
    let key = dispatch_key(&assignment.request);
    assert!(store.claim_request("execution", key, id).await.unwrap());
    store.prepare_assignment(&assignment, id).await.unwrap();
    if phase != "prepared" {
        store
            .save_receipt(
                "execution",
                key,
                &Receipt {
                    fingerprint: id.into(),
                    phase: phase.into(),
                    payload: json!(null),
                },
            )
            .await
            .unwrap();
    }
    assignment
}

#[tokio::test]
async fn only_prepared_receipts_replay_all_kinds_and_project_run_keys() {
    let store = FleetStore::open_memory().await.unwrap();
    let mut expected = Vec::new();
    for (id, kind, run) in [
        ("agent-a", "agent", None),
        ("brain-b", "brain", None),
        ("dag-c", "dag", None),
        ("operator-d", "operator", None),
        ("project-e", "project", Some("prun-current")),
        ("project-f", "project", None),
        ("project-g", "project", Some("project-g")),
        ("team-h", "team", None),
        ("todos-i", "todos", None),
    ] {
        expected.push(prepare(&store, id, kind, run, "prepared").await);
    }
    for phase in ["claimed", "accepted", "rejected"] {
        prepare(&store, &format!("dag-{phase}"), "dag", None, phase).await;
        prepare(
            &store,
            &format!("project-{phase}"),
            "project",
            Some(&format!("prun-{phase}")),
            phase,
        )
        .await;
    }
    // A receipt in another scope cannot re-open an accepted execution.
    store
        .claim_request("another-scope", "dag-accepted", "foreign")
        .await
        .unwrap();
    store
        .save_receipt(
            "another-scope",
            "dag-accepted",
            &Receipt {
                fingerprint: "foreign".into(),
                phase: "prepared".into(),
                payload: json!(null),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(store.pending_assignments("", 128).await.unwrap()).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[tokio::test]
async fn pending_pages_keep_global_order_cursor_and_batch_limit() {
    let store = FleetStore::open_memory().await.unwrap();
    let mut expected = Vec::new();
    // Insert in reverse order to make ordering independent of insertion order.
    for i in (0..140).rev() {
        let id = format!("dag-{i:03}");
        prepare(&store, &id, "dag", None, "prepared").await;
        expected.push(id);
    }
    for id in ["project-a", "project-b"] {
        prepare(
            &store,
            id,
            "project",
            Some(&format!("run-{id}")),
            "prepared",
        )
        .await;
        expected.push(id.into());
    }
    expected.sort();
    assert!(store.pending_assignments("", 0).await.unwrap().is_empty());
    let first = store.pending_assignments("", 999).await.unwrap();
    assert_eq!(first.len(), 128);
    let second = store
        .pending_assignments(&first.last().unwrap().index.id, 128)
        .await
        .unwrap();
    assert_eq!(second.len(), 14);
    let actual: Vec<_> = first
        .into_iter()
        .chain(second)
        .map(|assignment| assignment.index.id)
        .collect();
    assert_eq!(actual, expected);
    assert!(store
        .pending_assignments(actual.last().unwrap(), 128)
        .await
        .unwrap()
        .is_empty());
}

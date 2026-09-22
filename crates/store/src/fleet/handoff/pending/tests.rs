use super::*;
use crate::fleet::handoff::{dispatch_key, Receipt};
use serde_json::json;

fn assignment(id: &str, kind: &str, run: Option<&str>) -> Assignment {
    serde_json::from_value(json!({
        "index": {"id": id, "created_at": 1, "kind": kind,
            "node_id": "node-one", "status": "pending"},
        "request": {"id": id, "kind": kind, "input": {"run_id": run}},
        "definition": {"name": id}
    }))
    .unwrap()
}

async fn prepare(
    store: &FleetStore,
    id: &str,
    kind: &str,
    run: Option<&str>,
    phase: &str,
) -> Assignment {
    let assignment = assignment(id, kind, run);
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

#[tokio::test]
async fn invalid_index_request_pair_is_rejected_before_persisting() {
    for field in ["id", "kind"] {
        let store = FleetStore::open_memory().await.unwrap();
        let mut value = assignment("dag-invalid", "dag", None);
        if field == "id" {
            value.request.id = "dag-other".into();
        } else {
            value.request.kind = opencoder_core::fleet::ExecutionKind::Project;
        }
        let key = dispatch_key(&value.request);
        store
            .claim_request("execution", key, "input")
            .await
            .unwrap();
        let error = store.prepare_assignment(&value, "input").await.unwrap_err();
        assert!(error.to_string().contains("must match request"), "{error}");
        assert!(store.index(&value.index.id).await.unwrap().is_none());
        assert!(store.assignment(&value.index.id).await.unwrap().is_none());
        assert_eq!(
            store
                .receipt("execution", key)
                .await
                .unwrap()
                .unwrap()
                .phase,
            "claimed"
        );
    }
}

#[tokio::test]
async fn malformed_prepared_kind_is_an_explicit_error() {
    let store = FleetStore::open_memory().await.unwrap();
    prepare(&store, "dag-broken", "dag", None, "prepared").await;
    // Simulate a damaged persisted payload in this isolated in-memory store.
    store.conn.execute(
        "UPDATE execution_assignments SET assignment=json_set(assignment,'$.request.kind',NULL) WHERE id='dag-broken'",
        (),
    ).await.unwrap();
    assert!(store.pending_assignments("", 128).await.is_err());
}

//! Dispatch receipts, claim fingerprints and prepared-receipt lookup.
use crate::common::frozen;
use opencoder_core::fleet::ExecutionKind;
use opencoder_store::fleet::handoff::{dispatch_key, Receipt};
use opencoder_store::fleet::FleetStore;
use serde_json::json;

#[test]
fn dispatch_keys_use_run_ids_only_for_project_attempts() {
    let mut task = frozen("project-1", ExecutionKind::Project, 1);
    task.request.input = json!({"run_id": "run-first"});
    assert_eq!(dispatch_key(&task.request), "run-first");
    task.request.input = json!({});
    assert_eq!(dispatch_key(&task.request), "project-1");
    task.request.kind = ExecutionKind::Agent;
    task.request.input = json!({"run_id": "run-first"});
    assert_eq!(dispatch_key(&task.request), "project-1");
}

#[tokio::test]
async fn claim_request_keeps_the_first_fingerprint_and_reports_replays() {
    let store = FleetStore::open_memory().await.unwrap();
    assert!(store
        .claim_request("execution", "req-1", "fp-a")
        .await
        .unwrap());
    assert!(
        store
            .claim_request("execution", "req-1", "fp-a")
            .await
            .unwrap(),
        "an identical replay is accepted"
    );
    assert!(
        !store
            .claim_request("execution", "req-1", "fp-b")
            .await
            .unwrap(),
        "a different fingerprint conflicts instead of overwriting"
    );
    assert!(
        store.claim_request("other", "req-1", "fp-b").await.unwrap(),
        "scopes are independent"
    );
    let receipt = store.receipt("execution", "req-1").await.unwrap().unwrap();
    assert_eq!(receipt.fingerprint, "fp-a");
    assert_eq!(receipt.phase, "claimed");
    assert_eq!(receipt.payload, json!(null));
    assert!(store
        .receipt("execution", "missing")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn save_receipt_round_trips_and_rejects_fingerprint_changes() {
    let store = FleetStore::open_memory().await.unwrap();
    store
        .claim_request("execution", "req-1", "fp-a")
        .await
        .unwrap();
    let rejected = Receipt {
        fingerprint: "fp-a".into(),
        phase: "rejected".into(),
        payload: json!({"reason": "declined"}),
    };
    store
        .save_receipt("execution", "req-1", &rejected)
        .await
        .unwrap();
    let saved = store.receipt("execution", "req-1").await.unwrap().unwrap();
    assert_eq!(saved.fingerprint, "fp-a");
    assert_eq!(saved.phase, "rejected");
    assert_eq!(saved.payload, json!({"reason": "declined"}));
    let mut foreign = rejected;
    foreign.fingerprint = "fp-b".into();
    assert!(store
        .save_receipt("execution", "req-1", &foreign)
        .await
        .is_err());
    assert!(store
        .save_receipt("execution", "missing", &foreign)
        .await
        .is_err());
    assert_eq!(
        store
            .receipt("execution", "req-1")
            .await
            .unwrap()
            .unwrap()
            .phase,
        "rejected"
    );
}

#[tokio::test]
async fn pending_receipts_and_subject_lookup_only_expose_prepared_rows() {
    let store = FleetStore::open_memory().await.unwrap();
    store
        .claim_request("execution", "req-1", "fp")
        .await
        .unwrap();
    store
        .claim_request("execution", "req-2", "fp")
        .await
        .unwrap();
    let prepared = Receipt {
        fingerprint: "fp".into(),
        phase: "prepared".into(),
        payload: json!({"subject": "subject-2"}),
    };
    store
        .save_receipt("execution", "req-2", &prepared)
        .await
        .unwrap();

    let pending = store.pending_receipts("execution", "", 10).await.unwrap();
    assert_eq!(
        pending
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        ["req-2"]
    );
    let clamped = store.pending_receipts("execution", "", 0).await.unwrap();
    assert_eq!(
        clamped
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        ["req-2"],
        "limit 0 clamps to one prepared row"
    );
    assert_eq!(
        store
            .prepared_receipt_for("execution", "subject-2")
            .await
            .unwrap(),
        Some("req-2".to_owned())
    );
    assert!(store
        .prepared_receipt_for("execution", "subject-missing")
        .await
        .unwrap()
        .is_none());
}

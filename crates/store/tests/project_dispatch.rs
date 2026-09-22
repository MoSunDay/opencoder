use opencoder_core::fleet::*;
use opencoder_store::fleet::{
    handoff::{dispatch_key, Receipt},
    FleetStore,
};
use serde_json::json;

fn assignment(run: &str) -> Assignment {
    Assignment {
        private_context: None,
        runtime: None,
        codex: None,
        index: ExecutionIndex {
            id: "project-todo".into(),
            kind: ExecutionKind::Project,
            node_id: "node-original".into(),
            created_at: 17,
            status: ExecutionStatus::Pending,
        },
        request: CreateExecution {
            id: "project-todo".into(),
            kind: ExecutionKind::Project,
            target: Some("todo".into()),
            node_id: None,
            input: json!({"run_id":run,"action":"plan"}),
        },
        definition: Some(json!({"version":run})),
    }
}

#[test]
fn only_project_attempts_use_run_ids_for_dispatch_receipts() {
    let mut a = assignment("prun-first");
    assert_eq!(dispatch_key(&a.request), "prun-first");
    a.request.kind = ExecutionKind::Agent;
    assert_eq!(dispatch_key(&a.request), "project-todo");
    a.request.kind = ExecutionKind::Project;
    a.request.input = json!({});
    assert_eq!(dispatch_key(&a.request), "project-todo");
}

#[tokio::test]
async fn only_a_definitively_rejected_project_attempt_can_be_replaced_and_receipts_survive_reopen()
{
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fleet.db");
    let first = FleetStore::open(&path).await.unwrap();
    let second = FleetStore::open(&path).await.unwrap();
    let a = assignment("prun-first");
    let b = assignment("prun-second");
    assert!(first
        .claim_request("execution", dispatch_key(&a.request), "first")
        .await
        .unwrap());
    first.prepare_assignment(&a, "first").await.unwrap();
    assert!(second
        .claim_request("execution", dispatch_key(&b.request), "second")
        .await
        .unwrap());
    assert!(second
        .prepare_assignment(&b, "second")
        .await
        .unwrap_err()
        .to_string()
        .contains("unresolved"));
    let rejection = RpcReply::error(409, "project todo has no plan");
    first
        .save_receipt(
            "execution",
            dispatch_key(&a.request),
            &Receipt {
                fingerprint: "first".into(),
                phase: "rejected".into(),
                payload: json!(rejection),
            },
        )
        .await
        .unwrap();
    let mut wrong_owner = b.clone();
    wrong_owner.index.node_id = "node-new".into();
    assert!(second
        .prepare_assignment(&wrong_owner, "second")
        .await
        .unwrap_err()
        .to_string()
        .contains("ownership"));
    second.prepare_assignment(&b, "second").await.unwrap();
    let pending = first.pending_assignments("", 128).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].request, b.request);
    assert_eq!(pending[0].definition, b.definition);
    assert_eq!(
        first.index("project-todo").await.unwrap().unwrap().node_id,
        "node-original"
    );
    let accepted = RpcReply {
        status: 202,
        body: json!(b.index),
    };
    second
        .finish_dispatch(dispatch_key(&b.request), "second", &accepted)
        .await
        .unwrap();
    let c = assignment("prun-third");
    assert!(first
        .claim_request("execution", dispatch_key(&c.request), "third")
        .await
        .unwrap());
    assert!(first
        .prepare_assignment(&c, "third")
        .await
        .unwrap_err()
        .to_string()
        .contains("accepted"));
    drop(first);
    drop(second);
    let reopened = FleetStore::open(&path).await.unwrap();
    assert!(reopened
        .pending_assignments("", 128)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        reopened
            .receipt("execution", "prun-first")
            .await
            .unwrap()
            .unwrap()
            .payload,
        json!(rejection)
    );
    assert_eq!(
        reopened
            .receipt("execution", "prun-second")
            .await
            .unwrap()
            .unwrap()
            .payload,
        json!(accepted)
    );
    assert_eq!(
        reopened
            .assignment("project-todo")
            .await
            .unwrap()
            .unwrap()
            .request,
        b.request
    );
    assert!(!reopened
        .claim_request("execution", "prun-first", "different")
        .await
        .unwrap());
}

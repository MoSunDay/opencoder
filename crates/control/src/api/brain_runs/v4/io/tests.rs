use super::*;
use serde_json::json;

async fn state(root: &std::path::Path) -> Arc<AppState> {
    std::fs::create_dir_all(root.join("work")).unwrap();
    std::fs::write(
        root.join("work/opencoder.json"),
        json!({"agent":{"agents_dir":root.join("agents")}}).to_string(),
    )
    .unwrap();
    crate::new_state(root.join("work"), root.join("data"), None)
        .await
        .unwrap()
}

fn frozen(kind: ExecutionKind, id: &str) -> Assignment {
    let parent = json!({"run_id":"brain-old","operation_id":"old-operation"});
    let input = if kind == ExecutionKind::Brain {
        json!({"schema_version":7,"layered_intent":{"parent":parent}})
    } else {
        json!({"schema_version":7,"brain_layered":parent,"layered_inputs":{"result":"original"}})
    };
    Assignment {
        private_context: None,
        runtime: None,
        codex: None,
        definition: Some(json!({"version":"frozen"})),
        index: ExecutionIndex {
            id: id.into(),
            kind,
            node_id: "original-node".into(),
            status: ExecutionStatus::Pending,
            created_at: 1,
        },
        request: CreateExecution {
            id: id.into(),
            kind,
            target: None,
            input,
            node_id: Some("original-node".into()),
        },
    }
}

#[tokio::test]
async fn claim_only_roots_keep_the_original_wire_during_upgrade_and_rollback() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path()).await;
    for (id, version) in [("brain-claim-old", 0), ("brain-claim-new", 1)] {
        let mut original = frozen(ExecutionKind::Brain, id).request;
        mark(&mut original.input, version);
        let fingerprint = opencoder_core::token_hash(&serde_json::to_string(&original).unwrap());
        assert!(state
            .fleet
            .claim_request("execution", id, &fingerprint)
            .await
            .unwrap());
        let mut retry = original.clone();
        retry
            .input
            .as_object_mut()
            .unwrap()
            .remove("execution_io_version");
        mark(&mut retry.input, 1 - version);
        preserve_claim(&state, &mut retry).await.unwrap();
        assert_eq!(retry, original);
        assert!(state.fleet.assignment(id).await.unwrap().is_none());
        retry.input["changed"] = json!(true);
        assert!(preserve_claim(&state, &mut retry).await.is_err());
        assert!(state
            .fleet
            .claim_request("execution", id, &fingerprint)
            .await
            .unwrap());
    }
}

#[tokio::test]
async fn replay_legacy_children_preserves_fingerprint_inputs_and_owner() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path()).await;
    for (kind, id) in [
        (ExecutionKind::Agent, "agent-replay"),
        (ExecutionKind::Dag, "dag-replay"),
        (ExecutionKind::Brain, "brain-replay"),
    ] {
        let assignment = frozen(kind, id);
        let fingerprint =
            opencoder_core::token_hash(&serde_json::to_string(&assignment.request).unwrap());
        assert!(state
            .fleet
            .claim_request("execution", id, &fingerprint)
            .await
            .unwrap());
        state
            .fleet
            .prepare_assignment(&assignment, &fingerprint)
            .await
            .unwrap();
        let accepted = RpcReply {
            status: 202,
            body: json!(assignment.index),
        };
        state
            .fleet
            .finish_dispatch(id, &fingerprint, &accepted)
            .await
            .unwrap();
        let reply = replay(
            &state,
            "brain-old",
            "old-operation",
            &assignment.index.execution_ref(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(reply.status, 202, "{reply:?}");
        let body = if kind == ExecutionKind::Brain {
            &reply.body["execution"]
        } else {
            &reply.body
        };
        assert_eq!(body["node_id"], "original-node");
        assert_eq!(
            state.fleet.assignment(id).await.unwrap().unwrap().request,
            assignment.request
        );
        assert!(state
            .fleet
            .claim_request("execution", id, &fingerprint)
            .await
            .unwrap());
        let mut changed = assignment.request.clone();
        mark(&mut changed.input, 1);
        let changed_fingerprint =
            opencoder_core::token_hash(&serde_json::to_string(&changed).unwrap());
        assert!(!state
            .fleet
            .claim_request("execution", id, &changed_fingerprint)
            .await
            .unwrap());
        assert_eq!(
            replay(
                &state,
                "brain-other",
                "old-operation",
                &assignment.index.execution_ref()
            )
            .await
            .unwrap()
            .unwrap()
            .status,
            409
        );
    }
}

#[tokio::test]
async fn nested_admission_inherits_the_parent_contract_instead_of_the_new_default() {
    let root = tempfile::tempdir().unwrap();
    let state = state(root.path()).await;
    for (id, version) in [("brain-legacy", 0), ("brain-io", 1)] {
        let mut assignment = frozen(ExecutionKind::Brain, id);
        mark(&mut assignment.request.input, version);
        let fingerprint =
            opencoder_core::token_hash(&serde_json::to_string(&assignment.request).unwrap());
        assert!(state
            .fleet
            .claim_request("execution", id, &fingerprint)
            .await
            .unwrap());
        state
            .fleet
            .prepare_assignment(&assignment, &fingerprint)
            .await
            .unwrap();
        assert_eq!(
            admitted_version(&state, &json!({"parent":{"run_id":id}}))
                .await
                .unwrap(),
            version
        );
    }
    assert_eq!(
        admitted_version(&state, &json!({})).await.unwrap(),
        EXECUTION_IO_WRITER_VERSION
    );
    assert!(
        admitted_version(&state, &json!({"parent":{"run_id":"brain-missing"}}))
            .await
            .is_err()
    );
}

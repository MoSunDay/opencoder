use super::support::{spec, Harness};
use opencoder_core::brain::PlanVersion;
use opencoder_core::fleet::{
    Assignment, CreateExecution, ExecutionIndex, ExecutionKind, ExecutionStatus,
};
use opencoder_store::fleet::handoff::{dispatch_key, Receipt};
use serde_json::{json, Value};

#[tokio::test]
async fn bindings_saved_versions_and_pending_dispatch_prevent_removal() {
    let h = Harness::new().await;
    h.call("POST", "/api/dag/defs", spec("first")).await;
    h.state
        .fleet
        .put_definition(
            "capability_target",
            "registered",
            &json!({"kind":"dag","target":"first"}),
        )
        .await
        .unwrap();
    let blocked = h.call("DELETE", "/api/dag/defs/first", Value::Null).await;
    assert_eq!(blocked.0, 409, "{}", blocked.1);
    assert!(blocked.1["error"]
        .as_str()
        .unwrap()
        .contains("capability_target"));
    h.state
        .fleet
        .delete_definition("capability_target", "registered")
        .await
        .unwrap();
    // A saved immutable plan version binds the capability.
    h.state
        .fleet
        .save_brain_plan_document(&PlanVersion {
            id: "saved".into(),
            version: 1,
            plan: json!({"title":"Saved binding","nodes":[{"capability_id":"dag-first"}]}),
            changelog: "guard".into(),
            tags: Vec::new(),
            confidence: Default::default(),
            created_at: 1,
            author: "guard-test".into(),
        })
        .await
        .unwrap();
    let blocked = h.call("DELETE", "/api/dag/defs/first", Value::Null).await;
    assert_eq!(blocked.0, 409);
    assert!(blocked.1["error"]
        .as_str()
        .unwrap()
        .contains("plan:saved@1"));
    assert!(
        h.state.fleet.delete_brain_plan("saved").await.unwrap(),
        "plan removal must clear the saved version"
    );
    // A prepared dispatch receipt still references the definition.
    assert!(h
        .state
        .fleet
        .claim_request("execution", "pending", "hash")
        .await
        .unwrap());
    h.state
        .fleet
        .save_receipt(
            "execution",
            "pending",
            &Receipt {
                fingerprint: "hash".into(),
                phase: "prepared".into(),
                payload: json!({
                    "index":{"id":"dag-run","created_at":1,"kind":"dag",
                             "node_id":"node","status":"running"},
                    "request":{"kind":"dag","target":"first"}
                }),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        h.call("DELETE", "/api/dag/defs/first", Value::Null).await.0,
        409
    );
}

#[tokio::test]
async fn active_assignments_protect_resources_but_finished_history_is_unchanged() {
    let h = Harness::new().await;
    h.call("POST", "/api/dag/defs", spec("first")).await;
    // Seed a live dag dispatch through the fleet handoff APIs: claiming the
    // receipt freezes the assignment and publishes its index row in one
    // atomic prepare.
    let fleet = &h.state.fleet;
    let record = ExecutionIndex {
        id: "dag-run".into(),
        created_at: 1,
        kind: ExecutionKind::Dag,
        node_id: "node".into(),
        status: ExecutionStatus::Running,
    };
    let assignment = Assignment {
        private_context: None,
        runtime: None,
        codex: None,
        index: record.clone(),
        request: CreateExecution {
            id: record.id.clone(),
            kind: record.kind,
            target: Some("first".into()),
            input: Value::Null,
            node_id: None,
        },
        definition: Some(json!({"spec": spec("first")})),
    };
    assert!(fleet
        .claim_request(
            "execution",
            dispatch_key(&assignment.request),
            "guard-fingerprint"
        )
        .await
        .unwrap());
    fleet
        .prepare_assignment(&assignment, "guard-fingerprint")
        .await
        .unwrap();
    assert_eq!(
        h.call("DELETE", "/api/dag/defs/first", Value::Null).await.0,
        409
    );
    fleet
        .put_index(&ExecutionIndex {
            status: ExecutionStatus::Done,
            ..record
        })
        .await
        .unwrap();
    assert_eq!(
        h.call("DELETE", "/api/dag/defs/first", Value::Null).await.0,
        200
    );
    // Finished history keeps the frozen assignment untouched.
    let stored = fleet
        .assignment("dag-run")
        .await
        .unwrap()
        .expect("finished assignment history is retained");
    assert_eq!(
        serde_json::to_value(&stored).unwrap(),
        serde_json::to_value(&assignment).unwrap()
    );
}

#[tokio::test]
async fn deletion_waits_for_concurrent_reference_writer_and_then_rechecks() {
    let h = Harness::new().await;
    h.call("POST", "/api/dag/defs", spec("first")).await;
    let lock = h
        .state
        .fleet
        .shared_request_lock("catalog-maintenance", "registry")
        .await
        .unwrap();
    let delete = h.call("DELETE", "/api/dag/defs/first", Value::Null);
    tokio::pin!(delete);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut delete)
            .await
            .is_err()
    );
    h.state
        .fleet
        .put_definition(
            "capability_target",
            "new-reference",
            &json!({"kind":"dag","target":"first"}),
        )
        .await
        .unwrap();
    drop(lock);
    assert_eq!(delete.await.0, 409);
}

#[tokio::test]
async fn admission_and_reference_writes_wait_through_a_slow_catalog_mutation() {
    let h = Harness::new().await;
    h.call("POST", "/api/dag/defs", spec("first")).await;
    let lock = h
        .state
        .fleet
        .request_lock("catalog-maintenance", "registry")
        .await
        .unwrap();
    let app = h.app.clone();
    let schedule =
        tokio::spawn(async move {
            super::support::call(&app, "POST", "/api/schedules", json!({
            "id":"daily", "cron":"0 0 * * *", "kind":"dag", "target":"first", "enabled":false
        }), None).await
        });
    let app = h.app.clone();
    let admission = tokio::spawn(async move {
        super::support::call(
            &app,
            "POST",
            "/api/executions",
            json!({
                "id":"dag-lock-admission", "kind":"dag", "target":"first", "input":{}
            }),
            None,
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    assert!(
        !schedule.is_finished(),
        "reference write expired during the mutation"
    );
    assert!(
        !admission.is_finished(),
        "admission expired during the mutation"
    );
    drop(lock);
    assert_eq!(schedule.await.unwrap().0, 200);
    let admitted = admission.await.unwrap();
    assert_ne!(admitted.0, 500, "{}", admitted.1);
    assert!(
        !admitted.1.to_string().contains("目录正在更新"),
        "{}",
        admitted.1
    );
    assert!(
        !admitted.1.to_string().contains("deadline"),
        "{}",
        admitted.1
    );
}

#[tokio::test]
async fn disabled_schedules_and_agent_files_protect_objects_and_late_writes_fail() {
    let h = Harness::new().await;
    h.call("POST", "/api/dag/defs", spec("first")).await;
    let job =
        json!({"id":"daily","cron":"0 0 * * *","kind":"dag","target":"first","enabled":false});
    let created = h.call("POST", "/api/schedules", job.clone()).await;
    assert_eq!(created.0, 200, "{}", created.1);
    assert_eq!(
        h.call("DELETE", "/api/dag/defs/first", Value::Null).await.0,
        409
    );
    assert_eq!(
        h.call("DELETE", "/api/schedules/daily", Value::Null)
            .await
            .0,
        200
    );
    let root = h.dir.path().join("agents");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("instructions.md"), "Invoke dag-first when asked").unwrap();
    assert_eq!(
        h.call("DELETE", "/api/dag/defs/first", Value::Null).await.0,
        409
    );
    std::fs::remove_file(root.join("instructions.md")).unwrap();
    assert_eq!(
        h.call("DELETE", "/api/dag/defs/first", Value::Null).await.0,
        200
    );
    assert_eq!(h.call("POST", "/api/schedules", job.clone()).await.0, 409);
    assert_eq!(h.call("POST", "/api/dag/defs", spec("first")).await.0, 200);
    assert_eq!(h.call("POST", "/api/schedules", job).await.0, 200);
}

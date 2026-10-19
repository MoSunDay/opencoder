use super::support::{spec, Harness};
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
    let conn = turso::Builder::new_local(
        (h.dir.path().join("state/control.db"))
            .to_str()
            .expect("UTF-8 database path"),
    )
    .experimental_multiprocess_wal(true)
    .build()
    .await
    .unwrap()
    .connect()
    .unwrap();
    conn.execute(
        "INSERT INTO brain_plan_versions VALUES('saved',1,?1)",
        [json!({"plan":{"nodes":[{"capability_id":"dag-first"}]}}).to_string()],
    )
    .await
    .unwrap();
    let blocked = h.call("DELETE", "/api/dag/defs/first", Value::Null).await;
    assert_eq!(blocked.0, 409);
    assert!(blocked.1["error"]
        .as_str()
        .unwrap()
        .contains("plan:saved@1"));
    conn.execute("DELETE FROM brain_plan_versions", ())
        .await
        .unwrap();
    conn.execute(
        "INSERT INTO dispatch_receipts VALUES('execution','pending','hash','prepared',?1)",
        [json!({"request":{"kind":"dag","target":"first"}}).to_string()],
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
    let conn = turso::Builder::new_local(
        (h.dir.path().join("state/control.db"))
            .to_str()
            .expect("UTF-8 database path"),
    )
    .experimental_multiprocess_wal(true)
    .build()
    .await
    .unwrap()
    .connect()
    .unwrap();
    conn.execute(
        "INSERT INTO execution_index VALUES('dag-run',1,'dag','node','running')",
        (),
    )
    .await
    .unwrap();
    let assignment =
        json!({"request":{"kind":"dag","target":"first"},"definition":{"spec":spec("first")}})
            .to_string();
    conn.execute(
        "INSERT INTO execution_assignments VALUES('dag-run',?1)",
        [assignment.clone()],
    )
    .await
    .unwrap();
    assert_eq!(
        h.call("DELETE", "/api/dag/defs/first", Value::Null).await.0,
        409
    );
    conn.execute("UPDATE execution_index SET status='done'", ())
        .await
        .unwrap();
    assert_eq!(
        h.call("DELETE", "/api/dag/defs/first", Value::Null).await.0,
        200
    );
    let mut rows = conn
        .query(
            "SELECT assignment FROM execution_assignments WHERE id='dag-run'",
            (),
        )
        .await
        .unwrap();
    assert_eq!(
        rows.next()
            .await
            .unwrap()
            .unwrap()
            .get::<String>(0)
            .unwrap(),
        assignment
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

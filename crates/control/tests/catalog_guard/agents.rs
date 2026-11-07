use super::support::{call, Harness};
use serde_json::{json, Value};

async fn create(h: &Harness, name: &str) {
    let result = h.call("POST", "/api/agents", json!({"name":name})).await;
    assert_eq!(result.0, 201, "{}", result.1);
}

#[tokio::test]
async fn agent_conditional_delete_rechecks_card_and_preserves_legacy_calls() {
    let h = Harness::new().await;
    create(&h, "sample").await;
    let before = h.call("GET", "/api/agents/sample/meta", Value::Null).await;
    assert!(!before.2.is_empty());
    assert_eq!(
        h.call(
            "PUT",
            "/api/agents/sample",
            json!({"current":{"prompt":"pack"}})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(
            &h.app,
            "DELETE",
            "/api/agents/sample",
            Value::Null,
            Some(&before.2)
        )
        .await
        .0,
        412
    );
    let current = h.call("GET", "/api/agents/sample/meta", Value::Null).await;
    let deleted = call(
        &h.app,
        "DELETE",
        "/api/agents/sample",
        Value::Null,
        Some(&current.2),
    )
    .await;
    assert_eq!(deleted.0, 200, "{}", deleted.1);
    assert_eq!(
        h.call("DELETE", "/api/agents/sample", Value::Null).await.0,
        404
    );
    create(&h, "sample").await;
    assert_eq!(
        h.call("DELETE", "/api/agents/sample", Value::Null).await.0,
        200
    );
    assert_eq!(
        h.call("DELETE", "/api/agents/act", Value::Null).await.0,
        400
    );
}

#[tokio::test]
async fn agent_saved_alias_and_active_execution_block_delete_without_touching_history() {
    let h = Harness::new().await;
    create(&h, "sample").await;
    h.state
        .fleet
        .put_definition(
            "brain_plan",
            "saved",
            &json!({"capability_id":"agent-sample"}),
        )
        .await
        .unwrap();
    let blocked = h.call("DELETE", "/api/agents/sample", Value::Null).await;
    assert_eq!(blocked.0, 409, "{}", blocked.1);
    assert_eq!(
        blocked.1["details"]["references"][0],
        "definition:brain_plan:saved"
    );
    h.state
        .fleet
        .delete_definition("brain_plan", "saved")
        .await
        .unwrap();
    let path = h.dir.path().join("state/control.db");
    let conn = turso::Builder::new_local(path.to_str().unwrap())
        .experimental_multiprocess_wal(true)
        .build()
        .await
        .unwrap()
        .connect()
        .unwrap();
    conn.execute(
        "INSERT INTO execution_index VALUES('run',1,'agent','node','running')",
        (),
    )
    .await
    .unwrap();
    let assignment = json!({"index":{"id":"run","created_at":1,"kind":"agent","node_id":"node","status":"running"},"request":{"kind":"agent","target":"sample"}}).to_string();
    conn.execute(
        "INSERT INTO execution_assignments VALUES('run',?1)",
        [assignment.clone()],
    )
    .await
    .unwrap();
    assert_eq!(
        h.call("DELETE", "/api/agents/sample", Value::Null).await.0,
        409
    );
    conn.execute("UPDATE execution_index SET status='done'", ())
        .await
        .unwrap();
    assert_eq!(
        h.call("DELETE", "/api/agents/sample", Value::Null).await.0,
        200
    );
    let mut rows = conn
        .query(
            "SELECT assignment FROM execution_assignments WHERE id='run'",
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
async fn agent_delete_waits_for_reference_writer_and_rechecks() {
    let h = Harness::new().await;
    create(&h, "sample").await;
    let lock = h
        .state
        .fleet
        .shared_request_lock("catalog-maintenance", "registry")
        .await
        .unwrap();
    let delete = h.call("DELETE", "/api/agents/sample", Value::Null);
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
            "registered",
            &json!({"kind":"operator","target":"sample"}),
        )
        .await
        .unwrap();
    drop(lock);
    assert_eq!(delete.await.0, 409);
}

#[tokio::test]
async fn agent_removal_rejects_late_schedule_dag_and_admission_until_recreated() {
    let h = Harness::new().await;
    create(&h, "sample").await;
    assert_eq!(
        h.call("DELETE", "/api/agents/sample", Value::Null).await.0,
        200
    );
    let schedule =
        json!({"id":"daily","cron":"0 0 * * *","kind":"agent","target":"sample","enabled":false});
    assert_eq!(
        h.call("POST", "/api/schedules", schedule.clone()).await.0,
        409
    );
    let dag = json!({"name":"flow","steps":[{"name":"run","kind":{"type":"agent","agent":"sample","prompt":"inspect"}}]});
    assert_eq!(h.call("POST", "/api/dag/defs", dag).await.0, 409);
    let denied=h.call("POST","/api/executions",json!({"id":"agent-late","kind":"agent","target":"sample","input":{"prompt":"inspect"}})).await;
    assert_eq!(denied.0, 409, "{}", denied.1);
    assert!(h
        .state
        .fleet
        .assignment("agent-late")
        .await
        .unwrap()
        .is_none());
    create(&h, "sample").await;
    assert_eq!(h.call("POST", "/api/schedules", schedule).await.0, 200);
}

#[tokio::test]
async fn agent_resource_scan_distinguishes_pool_identity_and_protects_unreadable_files() {
    let h = Harness::new().await;
    create(&h, "sample").await;
    let pool = h.dir.path().join("agents/prompts/sample");
    std::fs::create_dir_all(&pool).unwrap();
    std::fs::write(
        pool.join("meta.json"),
        json!({"name":"sample","current":0,"history":[]}).to_string(),
    )
    .unwrap();
    let text = pool.join("instructions");
    std::fs::write(&text, "invoke agent-sample").unwrap();
    assert_eq!(
        h.call("DELETE", "/api/agents/sample", Value::Null).await.0,
        409
    );
    std::fs::write(&text, [0xff, 0]).unwrap();
    assert_eq!(
        h.call("DELETE", "/api/agents/sample", Value::Null).await.0,
        409
    );
    std::fs::remove_file(text).unwrap();
    let deleted = h.call("DELETE", "/api/agents/sample", Value::Null).await;
    assert_eq!(deleted.0, 200, "{}", deleted.1);
    assert!(pool.join("meta.json").exists());
}

#[tokio::test]
async fn agent_file_writer_lock_is_held_through_reference_validation() {
    let h = Harness::new().await;
    create(&h, "sample").await;
    let root = h.dir.path().join("agents");
    let held = opencoder_core::agent::scope::with_root_sync(Some(root), || {
        opencoder_agents::resources::lock::write_lock()
    })
    .unwrap();
    let app = h.app.clone();
    let delete =
        tokio::spawn(
            async move { call(&app, "DELETE", "/api/agents/sample", Value::Null, None).await },
        );
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(!delete.is_finished());
    // The writer completes before removal reads the card and its references.
    h.state
        .fleet
        .put_definition("team", "group", &json!({"members":[{"agent":"sample"}]}))
        .await
        .unwrap();
    drop(held);
    assert_eq!(delete.await.unwrap().0, 409);
}

#[tokio::test]
async fn agent_card_pool_links_do_not_block_card_removal_but_unknown_dependencies_do() {
    let h = Harness::new().await;
    create(&h, "sample").await;
    create(&h, "consumer").await;
    assert_eq!(
        h.call(
            "PUT",
            "/api/agents/consumer",
            json!({"current":{"prompt":"sample"}})
        )
        .await
        .0,
        200
    );
    let metadata = h.dir.path().join("agents/consumer/meta.json");
    let mut value: Value = serde_json::from_slice(&std::fs::read(&metadata).unwrap()).unwrap();
    value["future_dependency"] = json!("agent-sample");
    std::fs::write(&metadata, value.to_string()).unwrap();
    assert_eq!(
        h.call("DELETE", "/api/agents/sample", Value::Null).await.0,
        409
    );
    value.as_object_mut().unwrap().remove("future_dependency");
    std::fs::write(&metadata, value.to_string()).unwrap();
    let removed = h.call("DELETE", "/api/agents/sample", Value::Null).await;
    assert_eq!(removed.0, 200, "{}", removed.1);
    assert_eq!(
        h.call("GET", "/api/agents/consumer/meta", Value::Null)
            .await
            .1["meta"]["current"]["prompt"],
        "sample"
    );
}

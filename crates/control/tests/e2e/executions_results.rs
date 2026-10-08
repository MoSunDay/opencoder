use crate::support::Harness;
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus, RpcReply};
use reqwest::Method;
use serde_json::{json, Value};

async fn omitted_team() -> std::sync::Arc<Harness> {
    let h = Harness::new().await;
    h.put_index("team-long", ExecutionKind::Team, ExecutionStatus::Done)
        .await;
    h.node.set_inspect(
        "team-long",
        json!({"execution":{"status":"done"},
        "result":{},"topic":{"omitted":true}}),
    );
    h
}

#[tokio::test]
async fn long_native_conclusion_reads_all_chunks_without_project_cache_writes() {
    let h = omitted_team().await;
    let text = format!("native evidence {} end", "界".repeat(50_000));
    h.node.set_field_bytes(
        "team-long",
        "team.topic",
        serde_json::to_vec(&json!({"final_summary":text})).unwrap(),
    );
    let (created, todo) = h
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({"title":"read result","draft":"read native execution"})),
        )
        .await;
    assert_eq!(created, 200, "{todo}");
    let path = format!(
        "/api/project/todos/{}/executions",
        todo["id"].as_str().unwrap()
    );
    assert_eq!(
        h.req(
            Method::POST,
            &path,
            Some(json!({"execution_id":"team-long"}))
        )
        .await
        .0,
        200
    );
    let (_, before) = h.req(Method::GET, &path, None).await;

    let (code, result) = h
        .req(Method::GET, "/api/executions/team-long/result", None)
        .await;

    assert_eq!(code, 200, "{result}");
    let summary = result["summary"].as_str().unwrap();
    assert!(summary.starts_with("native evidence"));
    assert!(summary.len() <= 64 * 1024);
    assert_eq!(result["truncated"], true);
    assert_eq!(h.node.field_read_offsets(), vec![0, 65536, 131072]);
    assert_eq!(h.req(Method::GET, &path, None).await.1, before);
    assert!(before["assignments"][0].get("result_md").is_none());
    assert!(before["assignments"][0].get("sync_state").is_none());
    let database = h.data_dir().join("definitions.db");
    assert!(database.is_file());
    let store = opencoder_store::LibsqlStore::open(database).await.unwrap();
    let conn = store.conn().await.unwrap();
    let mut rows = conn
        .query("SELECT version FROM schema_version", ())
        .await
        .unwrap();
    assert_eq!(
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
        33
    );
    let mut rows = conn
        .query("PRAGMA table_info(project_todo_executions)", ())
        .await
        .unwrap();
    let mut columns = Vec::new();
    while let Some(row) = rows.next().await.unwrap() {
        columns.push(row.get::<String>(1).unwrap());
    }
    assert!(!columns
        .iter()
        .any(|name| matches!(name.as_str(), "result_md" | "sync_state")));
}

#[tokio::test]
async fn failed_chunk_read_is_visible_and_a_fresh_read_can_retry() {
    let h = omitted_team().await;
    h.node.set_field_bytes(
        "team-long",
        "team.topic",
        serde_json::to_vec(&json!({"final_summary":"evidence ".repeat(12_000)})).unwrap(),
    );
    h.node.fail_field_once("team-long", "team.topic", 65536);
    let (code, error) = h
        .req(Method::GET, "/api/executions/team-long/result", None)
        .await;
    assert_eq!(code, 503, "{error}");
    assert!(error["summary"].is_null());
    let (code, result) = h
        .req(Method::GET, "/api/executions/team-long/result", None)
        .await;
    assert_eq!(code, 200, "{result}");
    assert!(result["summary"].as_str().unwrap().starts_with("evidence"));
    assert_eq!(h.node.field_read_offsets(), vec![0, 65536, 0, 65536]);
}

#[tokio::test]
async fn dag_native_details_keep_all_sixty_four_steps() {
    let h = Harness::new().await;
    h.put_index("dag-complete", ExecutionKind::Dag, ExecutionStatus::Done)
        .await;
    let steps: Vec<Value> = (0..64)
        .map(|id| json!({"step_id":format!("step-{id}"),"status":"done"}))
        .collect();
    h.node
        .set_dag_step("dag-complete", "", RpcReply::ok(json!({"steps":steps})));
    h.node.set_inspect(
        "dag-complete",
        json!({"execution":{"status":"done"},"result":{},"dag_steps":{"steps":steps}}),
    );
    let (code, result) = h
        .req(Method::GET, "/api/executions/dag-complete/result", None)
        .await;
    assert_eq!(code, 200, "{result}");
    assert_eq!(result["steps"].as_array().unwrap().len(), 64);
    let (code, details) = h
        .req(Method::GET, "/api/dag/runs/dag-complete/progress", None)
        .await;
    assert_eq!(code, 200, "{details}");
    assert_eq!(details["steps"].as_array().unwrap().len(), 64);
}

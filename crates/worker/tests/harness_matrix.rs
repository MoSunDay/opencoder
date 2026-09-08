#![cfg(unix)]
#[path = "harness/fixture.rs"]
mod fixture;
mod support;
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use support::*;
#[path = "harness/cancel.rs"]
mod cancel;
#[path = "harness/mixed.rs"]
mod mixed;

// A dedicated integration binary owns its environment. One test runs every
// orchestrator sequentially so PATH/resource configuration cannot race.
#[tokio::test]
async fn codex_orchestrators_without_native_credentials() {
    let root = tempfile::tempdir().unwrap();
    let _config_home = opencoder_core::config::scoped_config_home(root.path().join("config-home"));
    let _environment = fixture::Environment::new(root.path());
    assert_eq!(
        opencoder_core::harness::agent_harness("act"),
        opencoder_core::harness::Harness::Codex
    );
    let node = fixture::node(&root.path().join("pure"), None).await;
    project(&node).await;
    team(&node).await;
    dag(&node).await;
    todos(&node).await;
    cancel::all(&node, root.path()).await;
    node.shutdown().await.unwrap();
    mixed::all(root.path()).await;
    let records = fixture::captures(root.path());
    assert!(records.len() >= 10, "{records:?}");
    let plans: Vec<_> = records
        .iter()
        .filter_map(|record| record["prompt"].as_str())
        .filter(|prompt| prompt.contains("你是一名资深工程规划助手"))
        .collect();
    assert!(!plans.is_empty());
    assert!(
        plans
            .iter()
            .all(|prompt| !prompt.contains("OPENCODER_DELIVERABLE_MANIFEST=")),
        "planning must not embed a run-specific delivery path"
    );
}

async fn create(
    node: &opencoder_worker::Worker,
    id: &str,
    kind: ExecutionKind,
    input: Value,
    definition: Value,
) -> Value {
    let mut a = assignment(node, id, kind, input, Some(definition));
    if kind == ExecutionKind::Project {
        a.request.target = Some("matrix-todo".into());
    }
    let reply = node.handle(NodeOperation::Create { assignment: a }).await;
    assert_eq!(reply.status, 200, "{reply:?}");
    settled(node, id).await
}
async fn project(node: &opencoder_worker::Worker) {
    let snapshot = project_snapshot("matrix-todo");
    let result = create(
        node,
        "project-matrix-todo",
        ExecutionKind::Project,
        json!({"action":"plan","run_id":"prun-matrix-plan"}),
        snapshot,
    )
    .await;
    assert_eq!(
        result["execution"]["status"], "idle",
        "Project Plan: {result}"
    );
    let mut session = Value::Null;
    for n in 0..3 {
        let reply = node
            .handle(NodeOperation::Command {
                execution: ExecutionRef {
                    id: "project-matrix-todo".into(),
                    kind: ExecutionKind::Project,
                },
                command: ExecutionCommand {
                    action: "execute".into(),
                    input: json!({"run_id":format!("prun-matrix-execute-{n}")}),
                },
            })
            .await;
        assert_eq!(reply.status, 200, "{reply:?}");
        let result = settled(node, "project-matrix-todo").await;
        assert_eq!(
            result["execution"]["status"], "idle",
            "Project Execute: {result}"
        );
        let run = &result["result"]["run"];
        assert_eq!(run["output_md"], "MATRIX_ANSWER");
        let input: Value = serde_json::from_str(run["input_snapshot"].as_str().unwrap()).unwrap();
        assert_eq!(input["harness"], "codex");
        assert!(input["model"].is_null());
        assert_eq!(input["model_source"], "codex_config");
        let trace: Value = serde_json::from_str(run["trace_manifest"].as_str().unwrap()).unwrap();
        assert_eq!(trace["harness"], "codex");
        assert!(trace["thread_id"].is_string());
        assert_eq!(trace["model_calls"], 0);
        assert_eq!(trace["artifacts"].as_array().unwrap().len(), 1);
        if n == 0 {
            session = run["session_id"].clone();
        } else {
            assert_eq!(
                run["session_id"], session,
                "unchanged Codex agent must resume"
            );
        }
    }
}
async fn team(node: &opencoder_worker::Worker) {
    let spec = json!({"name":"matrix-team","captain":"captain","members":[{"id":"captain","agent":"act","role":"coordinate"},{"id":"member","agent":"plan","role":"inspect"}]});
    let result = create(
        node,
        "team-matrix",
        ExecutionKind::Team,
        json!({"prompt":"MATRIX_TEAM"}),
        spec,
    )
    .await;
    assert_eq!(result["execution"]["status"], "done", "Team: {result}");
    assert_eq!(result["topic"]["final_summary"], "MATRIX_TEAM_DONE");
}
async fn dag(node: &opencoder_worker::Worker) {
    let spec = json!({"name":"matrix-dag","steps":[{"name":"first","kind":{"type":"agent","agent":"plan","prompt":"MATRIX_DAG_FIRST"}},{"name":"second","depends_on":["first"],"kind":{"type":"agent","agent":"act","prompt":"MATRIX_DAG_SECOND"}}]});
    let result = create(node, "dag-matrix", ExecutionKind::Dag, json!({}), spec).await;
    assert_eq!(result["execution"]["status"], "done", "DAG: {result}");
}
async fn todos(node: &opencoder_worker::Worker) {
    let spec = json!({"schema_version":1,"id":"wf-matrix","name":"matrix","objective":"finish item","constraints":[],"todos":[{"id":"t1","title":"step","requirement_background":"test","instructions":"MATRIX_CANDIDATE","depends_on":[],"agent":"act","max_attempts":2,"acceptance":{"criteria":"candidate exists"}}]});
    let result = create(node, "todos-matrix", ExecutionKind::Todos, json!({}), spec).await;
    assert_eq!(result["execution"]["status"], "done", "TODO: {result}");
    assert_eq!(result["workflow"]["items"][0]["status"], "passed");
}

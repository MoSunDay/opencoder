use super::*;
use axum::{
    extract::{OriginalUri, State},
    routing::any,
    Json, Router,
};
use std::sync::{Arc, Mutex};

struct NoModel;
impl opencoder_llm::ChatStream for NoModel {
    fn chat_stream(
        &self,
        _: opencoder_llm::ChatRequest,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<opencoder_llm::LlmEvent>> {
        anyhow::bail!("No model calls are allowed in transport tests")
    }
}

#[tokio::test]
async fn host_claim_uses_live_generation_and_writes_only_private_scoped_transport() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workflow");
    let input = json!({"device_count":1,"case_ids":["SnLW0E-032"],"case_source":"/frozen/cases.json","session_id":"forged"});
    let spec = opencoder_dag::devices::definition(&input).unwrap();
    std::fs::create_dir_all(root.join("dag-fixture")).unwrap();
    std::fs::write(root.join("dag-fixture/input.json"), input.to_string()).unwrap();
    let agents = temp.path().join("agents");
    std::fs::create_dir_all(agents.join("device-cases")).unwrap();
    std::fs::write(
        agents.join("device-cases/meta.json"),
        r#"{"name":"device-cases","harness":"codex"}"#,
    )
    .unwrap();
    let token = temp.path().join("root-token");
    std::fs::write(&token, "fixture-admin-secret").unwrap();
    type Seen = Arc<Mutex<Vec<(String, Value)>>>;
    async fn respond(
        State(seen): State<Seen>,
        OriginalUri(uri): OriginalUri,
        body: Option<Json<Value>>,
    ) -> Json<Value> {
        let value = body.map(|b| b.0).unwrap_or(Value::Null);
        seen.lock().unwrap().push((uri.path().into(), value));
        Json(if uri.path().ends_with("/status") {
            json!({"dag_id":"dag-fixture","target_step":"execute","assignments":[{"instance_id":"0","machine":"win-19","generation":8}]})
        } else if uri.path().ends_with("/claims") {
            json!({"capability":"fixture-step-secret","generation":9,"machine":"win-19","session_id":"host-generated","reservation_id":"abc123","recovery_only":true,"works":[{"native_run_id":"old-work"}]})
        } else {
            json!({"ok":true})
        })
    }
    let seen: Seen = Default::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .fallback(any(respond))
        .with_state(seen.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut config = opencoder_core::Config::default();
    config.agent.agents_dir = Some(agents);
    config.dag.device_manager = Some(DagDeviceConfig {
        host_endpoint: endpoint.clone(),
        step_endpoint: endpoint,
        token_file: token,
    });
    let deps = ExecDeps {
        store: Arc::new(opencoder_store::LibsqlStore::open_memory().await.unwrap()),
        client: Arc::new(NoModel),
        workdir: temp.path().into(),
        config,
    };
    let ctx=StepCtx{run_id:"dag-fixture".into(),instance:Some(0),instance_input:Some(json!(json!({"instance_id":"0","machine":"win-19","reservation_id":"abc123","generation":1,"case_ids":["SnLW0E-032"]}).to_string())),step:spec.steps[1].clone(),spec,states:Default::default(),outputs:Default::default(),workflow_root:root.clone(),log:None,knowledge_root:None,ops:Default::default()};
    let access = prepare(&ctx, &deps, "host-generated")
        .await
        .unwrap()
        .unwrap();
    let file = access.root.join("transport.json");
    let transport: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(transport["identity"]["session_id"], "host-generated");
    assert_eq!(transport["assignment"]["generation"], 9);
    assert_eq!(transport["recovery_only"], true);
    assert_eq!(transport["works"][0]["native_run_id"], "old-work");
    assert!(transport["input"].get("session_id").is_none());
    assert!(!file.starts_with(root.join("dag-fixture")));
    assert!(!transport.to_string().contains("fixture-admin-secret"));
    assert!(!access
        .prompt("task".into(), false)
        .contains("fixture-step-secret"));
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].1,
        json!({"instance_id":"0","generation":8,"session_id":"host-generated"})
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&access.root)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    server.abort();
}

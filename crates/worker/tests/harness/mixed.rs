use super::*;
use opencoder_llm::{CompletedToolCall, LlmEvent, MockChatClient};
use opencoder_store::{LibsqlStore, Store};
use std::{path::Path, sync::Arc};

pub async fn all(root: &Path) {
    // DAG forwards structured output; the native step returns its declared JSON.
    let native = Arc::new(MockChatClient::new()
        .push_script(done("```json\n{\"value\":\"MATRIX_NATIVE\"}\n```"))
        .with_default(done(r#"{"question":"inspect","participants":["build"],"summary":"aligned","aligned":true,"complete":true,"final_summary":"MATRIX_MIXED_TEAM"}"#)));
    let node = fixture::node(&root.join("mixed"), Some(native.clone())).await;
    let before = fixture::captures(root).len();
    let spec = json!({"name":"mixed","steps":[{"name":"native","kind":{"type":"agent","agent":"command","prompt":"MATRIX_NATIVE"}},{"name":"codex","depends_on":["native"],"kind":{"type":"agent","agent":"act","prompt":"MATRIX_DAG_MIXED"}}]});
    let result = create(&node, "dag-mixed", ExecutionKind::Dag, json!({}), spec).await;
    assert_eq!(result["execution"]["status"], "done", "{result}");
    assert_eq!(native.call_count(), 1);
    let records = fixture::captures(root);
    assert_eq!(records.len(), before + 1);
    assert!(records.last().unwrap()["prompt"]
        .as_str()
        .unwrap()
        .contains("MATRIX_NATIVE"));

    let spec = json!({"name":"mixed-team","captain":"command","members":[{"agent":"command"},{"agent":"build"}]});
    let result = create(
        &node,
        "team-mixed",
        ExecutionKind::Team,
        json!({"prompt":"MATRIX_TEAM"}),
        spec,
    )
    .await;
    assert_eq!(result["execution"]["status"], "done", "{result}");
    assert!(fixture::captures(root).len() > records.len());
    assert!(native.call_count() > 1);
    node.shutdown().await.unwrap();
    mixed_todos(root).await;
    subagent(root).await;
    project_switch(root).await;
    plan_preflight(root).await;
}
fn done(text: &str) -> Vec<LlmEvent> {
    vec![
        LlmEvent::TextDelta(text.into()),
        LlmEvent::Completed {
            text: text.into(),
            tool_calls: vec![],
            usage: None,
        },
    ]
}
async fn subagent(root: &Path) {
    let client = Arc::new(
        MockChatClient::new()
            .push_script(vec![LlmEvent::Completed {
                text: "delegate".into(),
                tool_calls: vec![CompletedToolCall {
                    id: "child".into(),
                    name: "task".into(),
                    input: json!({"subagent_type":"build","prompt":"MATRIX_SUBAGENT"}),
                }],
                usage: None,
            }])
            .push_script(done("MATRIX_PARENT_DONE")),
    );
    let store = Arc::new(LibsqlStore::open_memory().await.unwrap());
    let config = opencoder_core::Config::default();
    let mut session = opencoder_session::SessionState::new(
        "matrix-parent",
        opencoder_core::resolve_agent("act").unwrap(),
        config,
        client.clone(),
        root.into(),
    )
    .with_store(store.clone());
    session.harness.harness = opencoder_core::harness::Harness::Opencoder;
    opencoder_session::run(&mut session, "delegate work".into(), |_| {})
        .await
        .unwrap();
    assert_eq!(
        client.call_count(),
        2,
        "child must not call native provider"
    );
    let tasks = store.list_subagent_tasks(&session.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].result.as_deref(), Some("MATRIX_ANSWER"));
    let child = &tasks[0].child_session_id;
    assert_eq!(
        store.harness_runtime(child).await.unwrap().unwrap().harness,
        opencoder_core::harness::Harness::Codex
    );
}
async fn project_switch(root: &Path) {
    let client = Arc::new(MockChatClient::new().with_default(done("MATRIX_NATIVE_PROJECT")));
    let node = fixture::node(&root.join("switch"), Some(client.clone())).await;
    project(&node).await;
    assert_eq!(client.call_count(), 0);
    let before = settled(&node, "project-matrix-todo").await["result"]["run"]["session_id"].clone();
    fixture::card(&root.join("agents"), "act", "opencoder");
    let reply = node
        .handle(NodeOperation::Command {
            execution: ExecutionRef {
                id: "project-matrix-todo".into(),
                kind: ExecutionKind::Project,
            },
            command: ExecutionCommand {
                action: "execute".into(),
                input: json!({"run_id":"prun-switched-native"}),
            },
        })
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    let result = settled(&node, "project-matrix-todo").await;
    assert_eq!(result["execution"]["status"], "idle", "{result}");
    assert_ne!(result["result"]["run"]["session_id"], before);
    assert_eq!(
        result["result"]["run"]["output_md"],
        "MATRIX_NATIVE_PROJECT"
    );
    assert_eq!(client.call_count(), 1);
    fixture::card(&root.join("agents"), "act", "codex");
    node.shutdown().await.unwrap();
}
async fn plan_preflight(root: &Path) {
    // Planning executes plan, even when the todo's eventual executor is native.
    fixture::card(&root.join("agents"), "act", "opencoder");
    let node = fixture::node(&root.join("plan-only"), None).await;
    let result = create(
        &node,
        "project-matrix-todo",
        ExecutionKind::Project,
        json!({"action":"plan","run_id":"prun-plan-only"}),
        project_snapshot("matrix-todo"),
    )
    .await;
    assert_eq!(result["execution"]["status"], "idle", "{result}");
    let reply = node
        .handle(NodeOperation::Command {
            execution: ExecutionRef {
                id: "project-matrix-todo".into(),
                kind: ExecutionKind::Project,
            },
            command: ExecutionCommand {
                action: "execute".into(),
                input: json!({"run_id":"prun-native-needs-key"}),
            },
        })
        .await;
    assert_eq!(
        reply.status, 400,
        "native execution must still validate credentials: {reply:?}"
    );
    fixture::card(&root.join("agents"), "act", "codex");
    node.shutdown().await.unwrap();
}

async fn mixed_todos(root: &Path) {
    for parent_native in [false, true] {
        let client = if parent_native {
            MockChatClient::new()
                .push_script(done(r#"{"operation":"dispatch","todos":[{"todo_id":"t1","context_mode":"new"}],"reason":"ready"}"#))
                .push_script(done(r#"{"operation":"accept","reason":"verified","mark_milestone":true}"#))
                .push_script(done(r#"{"operation":"complete","reason":"all accepted"}"#))
        } else {
            MockChatClient::new().push_script(done(r#"{"status":"candidate","summary":"done","result":"MATRIX_NATIVE","verification":"checked","evidence_refs":[],"recovery_context":{"summary":"done","refs":[]}}"#))
        };
        let client = Arc::new(client);
        fixture::card(
            &root.join("agents"),
            "workflow",
            if parent_native { "opencoder" } else { "codex" },
        );
        let node = fixture::node(
            &root.join(format!("mixed-todo-{parent_native}")),
            Some(client.clone()),
        )
        .await;
        let before = fixture::captures(root).len();
        let spec = json!({"schema_version":1,"id":"wf-mixed","name":"mixed","objective":"finish","constraints":[],"todos":[{"id":"t1","title":"step","requirement_background":"test","instructions":"MATRIX_CANDIDATE","depends_on":[],"agent":if parent_native {"act"} else {"command"},"max_attempts":2,"acceptance":{"criteria":"done"}}]});
        let result = create(&node, "todos-mixed", ExecutionKind::Todos, json!({}), spec).await;
        assert_eq!(result["execution"]["status"], "done", "{result}");
        assert_eq!(result["workflow"]["items"][0]["status"], "passed");
        assert_eq!(client.call_count(), if parent_native { 3 } else { 1 });
        assert_eq!(
            fixture::captures(root).len() - before,
            if parent_native { 1 } else { 3 }
        );
        node.shutdown().await.unwrap();
    }
    fixture::card(&root.join("agents"), "workflow", "codex");
}

mod support;
use opencoder_core::fleet::*;
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use std::sync::Arc;
use support::*;
fn done(text: &str) -> Vec<LlmEvent> {
    vec![LlmEvent::Completed {
        text: text.into(),
        tool_calls: vec![],
        usage: None,
    }]
}

#[tokio::test]
async fn managed_team_dag_todo_return_typed_downloadable_outputs() {
    let _config = support::isolated_config();
    let cases = [
        (
            ExecutionKind::Team,
            json!({"name":"review","captain":"captain","members":[{"id":"captain","agent":"act","role":"coordinate"},{"id":"member","agent":"act","role":"review"}]}),
            json!("team completed"),
        ),
        (
            ExecutionKind::Dag,
            json!({"name":"local-dag","steps":[{"name":"review","kind":{"type":"agent","prompt":"review result"}}]}),
            json!({"review":"node-owned answer"}),
        ),
        (
            ExecutionKind::Todos,
            json!({"schema_version":1,"id":"wf-test","name":"test","objective":"finish item","constraints":[],"todos":[{"id":"t1","title":"step","requirement_background":"test","instructions":"return candidate","depends_on":[],"agent":"act","max_attempts":2,"acceptance":{"criteria":"candidate exists"}}]}),
            json!({"t1":"ok"}),
        ),
    ];
    for (kind, definition, expected) in cases {
        let dir = tempfile::tempdir().unwrap();
        let client = match kind {
            ExecutionKind::Team => Arc::new(MockChatClient::new().with_default(done(r#"{"question":"inspect","participants":["member"],"summary":"aligned","aligned":true,"complete":true,"final_summary":"team completed"}"#))),
            ExecutionKind::Todos => Arc::new(MockChatClient::new()
                .push_script(done(r#"{"operation":"dispatch","todos":[{"todo_id":"t1","context_mode":"new"}],"reason":"ready"}"#))
                .push_script(done(r#"{"status":"candidate","summary":"done","result":"ok","verification":"checked","evidence_refs":[],"recovery_context":{"summary":"done","refs":[]}}"#))
                .push_script(done(r#"{"operation":"accept","reason":"meets criteria","mark_milestone":true}"#))
                .push_script(done(r#"{"operation":"complete","reason":"all passed"}"#))),
            _ => Arc::new(MockChatClient::new().with_default(vec![LlmEvent::TextDelta("node-owned answer".into()), LlmEvent::Completed { text:"node-owned answer".into(), tool_calls:vec![],usage:None }])),
        };
        let node = worker(dir.path(), client).await;
        let id = format!("{}-managed-output", kind.prefix());
        let schema = if kind == ExecutionKind::Team {
            json!({"type":"string"})
        } else {
            json!({"type":"object"})
        };
        let input = json!({"prompt":"Produce evidence", "_brain":{"parent":{"run_id":"brain-parent","node_id":node.registration().id,"instance_id":"review","attempt":1},"action":{"kind":kind,"target":"act","prompt":"Produce evidence","output_mode":if kind == ExecutionKind::Team {"text"} else {"json"}},"output_schema":schema}});
        let reply = node
            .handle(NodeOperation::Create {
                assignment: assignment(&node, &id, kind, input, Some(definition)),
            })
            .await;
        assert_eq!(reply.status, 200, "{reply:?}");
        let detail = settled(&node, &id).await;
        assert_eq!(detail["execution"]["status"], "done", "{detail}");
        let record: Value = serde_json::from_slice(
            &std::fs::read(
                dir.path()
                    .join(format!("node/{}/{id}/execution.json", kind.prefix())),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(record["result"]["brain_output"]["value"], expected);
        let artifacts = record["result"]["brain_output"]["artifacts"]
            .as_array()
            .unwrap();
        assert!(artifacts
            .iter()
            .any(|a| a["step"] == "brain-result" && a["sha256"].as_str().unwrap().len() == 64));
        let reply = node
            .handle(NodeOperation::Artifact {
                request: ArtifactRequest {
                    execution: ExecutionRef { id, kind },
                    step: "brain-result".into(),
                    file: "output.json".into(),
                    offset: 0,
                    version: None,
                },
            })
            .await;
        assert_eq!(reply.status, 200, "{reply:?}");
        assert_eq!(reply.body["eof"], true);
        node.shutdown().await.unwrap();
    }
}

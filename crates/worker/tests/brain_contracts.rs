#![cfg(not(windows))]
mod support;

use opencoder_core::fleet::*;
use opencoder_llm::{LlmEvent, MockChatClient};
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use std::sync::Arc;

async fn run_output(text: String, required: &[&str]) -> (Value, Value, RpcReply, Vec<RpcReply>) {
    let (_scope, _home) = support::isolated_config();
    let directory = tempfile::tempdir().unwrap();
    let model = MockChatClient::new().with_default(vec![LlmEvent::Completed {
        text,
        tool_calls: vec![],
        usage: None,
    }]);
    let node = support::worker(directory.path(), Arc::new(model)).await;
    let id = "operator-contract";
    let reference = ExecutionRef {
        id: id.into(),
        kind: ExecutionKind::Operator,
    };
    let input = json!({"schema_version":7,"prompt":"Return the task evidence",
        "brain_layered":{"run_id":"brain-contract","operation_id":"brain-contract#l1#work#a1",
            "layer":1,"node_id":"work","capability":{"required_outputs":required}}});
    let accepted = node
        .handle(NodeOperation::Create {
            assignment: support::assignment(&node, id, reference.kind, input, None),
        })
        .await;
    assert_eq!(accepted.status, 200, "{accepted:?}");
    let detail = support::settled(&node, id).await;
    let summary = node
        .handle(NodeOperation::Brain {
            execution: reference.clone(),
            action: "layered_summary".into(),
            input: Value::Null,
        })
        .await;
    assert_eq!(summary.status, 200, "{summary:?}");
    let mut pointers = Vec::new();
    for path in ["", "/", "/a~1b~0"] {
        pointers.push(
            node.handle(NodeOperation::Brain {
                execution: reference.clone(),
                action: "layered_output".into(),
                input: json!({"path":path}),
            })
            .await,
        );
    }
    let missing = node
        .handle(NodeOperation::Brain {
            execution: reference,
            action: "layered_output".into(),
            input: json!({"path":"/absent"}),
        })
        .await;
    node.shutdown().await.unwrap();
    (detail, summary.body, missing, pointers)
}

#[tokio::test]
async fn structured_business_failure_survives_summary_and_missing_pointer_is_actionable() {
    let output = json!({"summary":"Tests executed","passed":false,
        "failures":["expected 2, got 1"],"revision":"patch-1"});
    let (detail, summary, missing, _) =
        run_output(output.to_string(), &["passed", "revision"]).await;
    assert_eq!(detail["execution"]["status"], "done");
    assert_eq!(detail["result"]["scheduler_output"], output);
    assert_eq!(
        serde_json::from_str::<Value>(summary["summary"].as_str().unwrap()).unwrap(),
        output
    );
    assert_eq!(summary["truncated"], false);
    assert_eq!(missing.status, 422);
    assert!(missing.body.to_string().contains("/absent"));
}

#[tokio::test]
async fn missing_required_output_is_an_execution_failure_with_the_original_output_retained() {
    let output = json!({"summary":"finished"});
    let (detail, summary, _, _) = run_output(output.to_string(), &["passed"]).await;
    assert_eq!(detail["execution"]["status"], "error");
    assert_eq!(detail["result"]["scheduler_output"], output);
    assert!(summary["summary"]
        .as_str()
        .unwrap()
        .contains("required capability output field passed"));
}

#[tokio::test]
async fn oversized_evidence_reports_a_contract_failure_instead_of_hiding_a_late_verdict() {
    let output = json!({"summary":"x".repeat(17000),"passed":false});
    let (detail, summary, _, _) = run_output(output.to_string(), &["passed"]).await;
    assert_eq!(detail["execution"]["status"], "error");
    assert_eq!(detail["result"]["scheduler_output"]["passed"], false);
    let evidence: Value = serde_json::from_str(summary["summary"].as_str().unwrap()).unwrap();
    assert!(evidence["error"]
        .as_str()
        .unwrap()
        .contains("evidence exceeds"));
    assert_eq!(evidence["result_truncated"], true);
    assert!(summary["summary"].as_str().unwrap().len() < 16384);
}

#[tokio::test]
async fn complete_output_binding_uses_empty_pointer_and_slash_keeps_its_exact_meaning() {
    for output in [json!("completed task text"), json!({"passed":true})] {
        let (_, _, _, pointers) = run_output(
            output
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| output.to_string()),
            &[],
        )
        .await;
        assert_eq!(pointers[0].status, 200);
        assert_eq!(pointers[0].body["value"], output);
        assert_eq!(
            pointers[1].status, 422,
            "slash must not stand in for the root"
        );
    }
    let output = json!({"":"empty property", "a/b~":"escaped property", "passed":true});
    let (_, _, _, pointers) = run_output(output.to_string(), &[]).await;
    assert_eq!(pointers[0].body["value"], output);
    assert_eq!(pointers[1].status, 200);
    assert_eq!(pointers[1].body["value"], "empty property");
    assert_eq!(pointers[2].status, 200);
    assert_eq!(pointers[2].body["value"], "escaped property");
}

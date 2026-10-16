use super::{artifacts::HeldOutput, fixture::READER, support};
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::Semaphore;

#[tokio::test]
async fn worker_restart_keeps_frozen_inputs_outputs_and_file_references_without_duplicate_execution(
) {
    let (_config, _home) = support::isolated_config();
    let temp = tempfile::tempdir().unwrap();
    let gate = Arc::new(Semaphore::new(0));
    let node = support::worker(temp.path(), Arc::new(HeldOutput(gate.clone()))).await;
    let id = "operator-io-recovery";
    let assignment = support::assignment(
        &node,
        id,
        ExecutionKind::Operator,
        json!({"schema_version":7,"prompt":"return the frozen result",
            "layered_inputs":{"flag":false,"zero":0,"empty":[]},
            "execution_io_version":1,"brain_layered":{"run_id":"brain-recovery","operation_id":"brain-recovery#l1#work#a1",
                "layer":1,"node_id":"work","capability":{"required_outputs":["passed"]}}}),
        None,
    );
    assert_eq!(
        node.handle(NodeOperation::Create {
            assignment: assignment.clone()
        })
        .await
        .status,
        200
    );
    let dir = temp.path().join("node/operator").join(id).join("workspace");
    std::fs::write(dir.join("report.txt"), b"evidence").unwrap();
    std::fs::write(
        dir.join("artifacts.json"),
        json!({"files":[{"path":"report.txt","bytes":8,
        "sha256":format!("{:x}",Sha256::digest(b"evidence"))}]})
        .to_string(),
    )
    .unwrap();
    gate.add_permits(1);
    let before = support::settled(&node, id).await;
    assert_eq!(before["execution"]["status"], "done", "{before}");
    let indexes = node.indexes().await.unwrap();
    node.shutdown().await.unwrap();
    drop(node);
    let reopened = support::worker(temp.path(), support::mock()).await;
    assert_eq!(
        reopened
            .handle(NodeOperation::Create { assignment })
            .await
            .status,
        200
    );
    assert_eq!(reopened.indexes().await.unwrap(), indexes);
    let after = support::settled(&reopened, id).await;
    assert_eq!(after["request"]["input"], before["request"]["input"]);
    assert_eq!(after["result"], before["result"]);
    let reference = ExecutionRef {
        id: id.into(),
        kind: ExecutionKind::Operator,
    };
    let files = reopened
        .handle(NodeOperation::Brain {
            execution: reference.clone(),
            action: "layered_output".into(),
            input: json!({"source":"artifacts","path":""}),
        })
        .await;
    assert_eq!(files.status, 200);
    assert_eq!(files.body["value"], before["result"]["scheduler_artifacts"]);
    assert_eq!(
        reopened
            .handle(NodeOperation::Artifact {
                request: ArtifactRequest {
                    execution: reference,
                    step: "workspace".into(),
                    index: None,
                    file: "report.txt".into(),
                    offset: 0,
                    version: None,
                }
            })
            .await
            .status,
        200
    );
    reopened.shutdown().await.unwrap();
}

#[tokio::test]
async fn dag_resume_rejects_input_drift_and_keeps_the_original_execution_identity() {
    let (_config, _home) = support::isolated_config();
    let temp = tempfile::tempdir().unwrap();
    let (node, _container, _bridge) = support::dag_worker(temp.path(), support::mock()).await;
    let program = format!("#include <unistd.h>\nint main(void) {{ execl(\"/usr/bin/python3\",\"python3\",\"-c\",{},(char*)0); return 127; }}",
        serde_json::to_string(READER).unwrap());
    support::stage_binary(&temp.path().join("node"), "io-reader", &program);
    let id = "dag-io-drift";
    let definition = json!({"spec":{"name":"io-drift","steps":[
        {"name":"inspect","kind":{"type":"binary","resource":"io-reader"}}]}});
    let input = json!({"schema_version":7,"layered_inputs":{
        "payload":{"seed":"drift","flag":false,"zero":0,"empty":[],"revision":"rev-drift"},
        "origin":"frozen-origin"}, "execution_io_version":1,"brain_layered":{"run_id":"brain-drift",
        "operation_id":"brain-drift#l1#work#a1","layer":1,"node_id":"work",
        "capability":{"required_outputs":["missing"]}}});
    let reference = ExecutionRef {
        id: id.into(),
        kind: ExecutionKind::Dag,
    };
    let accepted = node
        .handle(NodeOperation::Create {
            assignment: support::assignment(
                &node,
                id,
                ExecutionKind::Dag,
                input.clone(),
                Some(definition),
            ),
        })
        .await;
    assert_eq!(accepted.status, 200, "{accepted:?}");
    let before = support::settled(&node, id).await;
    assert_eq!(before["execution"]["status"], "error", "{before}");
    assert!(before["result"]["scheduler_error"]
        .as_str()
        .unwrap()
        .contains("missing"));
    let path = support::native::run_root(&temp.path().join("node"), id).join("input.json");
    let mut changed = input["layered_inputs"].clone();
    changed["origin"] = json!("changed-origin");
    std::fs::write(&path, changed.to_string()).unwrap();
    let resumed = node
        .handle(NodeOperation::Command {
            execution: reference,
            command: ExecutionCommand {
                action: "resume".into(),
                input: Value::Null,
            },
        })
        .await;
    assert_eq!(resumed.status, 200, "{resumed:?}");
    let after = support::settled(&node, id).await;
    assert_eq!(after["execution"]["status"], "error", "{after}");
    assert_eq!(after["execution"]["id"], id);
    assert_eq!(after["request"]["input"], before["request"]["input"]);
    assert!(
        after["error"].as_str().unwrap().contains("changed"),
        "{after}"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(path).unwrap()).unwrap(),
        changed
    );
    node.shutdown().await.unwrap();
}

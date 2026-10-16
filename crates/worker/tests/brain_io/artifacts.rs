use super::support;
use base64::Engine;
use opencoder_core::fleet::*;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent};
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};
use tokio::sync::Semaphore;

pub(super) struct HeldOutput(pub Arc<Semaphore>);
impl ChatStream for HeldOutput {
    fn chat_stream(&self, _: ChatRequest) -> anyhow::Result<tokio::sync::mpsc::Receiver<LlmEvent>> {
        let gate = self.0.clone();
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        tokio::spawn(async move {
            gate.acquire().await.unwrap().forget();
            let _ = sender
                .send(LlmEvent::Completed {
                    text: "{\"passed\":true}".into(),
                    tool_calls: vec![],
                    usage: None,
                })
                .await;
        });
        Ok(receiver)
    }
}

async fn execute(
    prepare: impl FnOnce(&Path),
    check: impl AsyncFnOnce(opencoder_worker::Worker, Value, std::path::PathBuf),
) {
    let (_config, _home) = support::isolated_config();
    let temp = tempfile::tempdir().unwrap();
    let gate = Arc::new(Semaphore::new(0));
    let node = support::worker(temp.path(), Arc::new(HeldOutput(gate.clone()))).await;
    let id = "operator-io-files";
    let input = json!({"schema_version":7,"execution_io_version":1,"prompt":"return evidence",
        "brain_layered":{"run_id":"brain-files","operation_id":"brain-files#l1#work#a1",
            "layer":1,"node_id":"work","capability":{"required_outputs":["passed"]}}});
    let accepted = node
        .handle(NodeOperation::Create {
            assignment: support::assignment(&node, id, ExecutionKind::Operator, input, None),
        })
        .await;
    assert_eq!(accepted.status, 200, "{accepted:?}");
    let dir = temp.path().join("node/operator").join(id).join("workspace");
    prepare(&dir);
    gate.add_permits(1);
    let detail = support::settled(&node, id).await;
    check(node.clone(), detail, dir).await;
    node.shutdown().await.unwrap();
}

fn write_declaration(dir: &Path, path: &str, content: &[u8]) {
    std::fs::write(
        dir.join("artifacts.json"),
        json!({"files":[{
            "path":path,"bytes":content.len(),"sha256":format!("{:x}",Sha256::digest(content))
        }]})
        .to_string(),
    )
    .unwrap();
}

fn request(file: &Value) -> ArtifactRequest {
    ArtifactRequest {
        execution: serde_json::from_value(file["execution"].clone()).unwrap(),
        step: file["step"].as_str().unwrap().into(),
        index: file["index"].as_u64().map(|index| index as usize),
        file: file["file"].as_str().unwrap().into(),
        offset: 0,
        version: None,
    }
}

#[tokio::test]
async fn declared_operator_file_is_bound_by_identity_and_tampering_is_rejected() {
    execute(
        |dir| {
            std::fs::write(dir.join("report.txt"), b"good").unwrap();
            std::fs::write(dir.join("undeclared.txt"), b"hidden").unwrap();
            write_declaration(dir, "report.txt", b"good");
            let path = dir.join("artifacts.json");
            let mut declaration: Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let row = declaration["files"][0].as_object_mut().unwrap();
            let file = row.remove("path").unwrap();
            row.insert("file".into(), file);
            std::fs::write(path, declaration.to_string()).unwrap();
        },
        async |node, detail, dir| {
            assert_eq!(detail["execution"]["status"], "done", "{detail}");
            let files = &detail["result"]["scheduler_artifacts"];
            assert_eq!(files.as_array().unwrap().len(), 1);
            assert_eq!(files[0]["execution"]["id"], "operator-io-files");
            let binding = node
                .handle(NodeOperation::Brain {
                    execution: request(&files[0]).execution,
                    action: "layered_output".into(),
                    input: json!({"source":"artifacts","path":"/0"}),
                })
                .await;
            assert_eq!(binding.status, 200);
            assert_eq!(binding.body["value"], files[0]);
            let read = node
                .handle(NodeOperation::Artifact {
                    request: request(&files[0]),
                })
                .await;
            assert_eq!(read.status, 200);
            assert_eq!(
                base64::engine::general_purpose::STANDARD
                    .decode(read.body["bytes_b64"].as_str().unwrap())
                    .unwrap(),
                b"good"
            );
            let mut unknown = request(&files[0]);
            unknown.file = "undeclared.txt".into();
            assert_eq!(
                node.handle(NodeOperation::Artifact { request: unknown })
                    .await
                    .status,
                400
            );
            std::fs::write(dir.join("report.txt"), b"evil").unwrap();
            let tampered = node
                .handle(NodeOperation::Artifact {
                    request: request(&files[0]),
                })
                .await;
            assert_eq!(tampered.status, 409);
            assert!(tampered.body.to_string().contains("SHA-256"));
        },
    )
    .await;
}

#[tokio::test]
async fn report_archive_path_is_indexed_and_downloaded_with_the_declared_digest() {
    execute(
        |dir| {
            std::fs::write(dir.join("report.bin"), b"raw").unwrap();
            std::fs::write(
                dir.join("output.json"),
                json!({"report_archive":{
                    "path":"report.bin","bytes":3,"sha256":format!("{:x}",Sha256::digest(b"raw"))
                }})
                .to_string(),
            )
            .unwrap();
        },
        async |node, detail, _| {
            assert_eq!(detail["execution"]["status"], "done", "{detail}");
            let files = &detail["result"]["scheduler_artifacts"];
            assert_eq!(files.as_array().unwrap().len(), 1);
            let read = node
                .handle(NodeOperation::Artifact {
                    request: request(&files[0]),
                })
                .await;
            assert_eq!(read.status, 200, "{read:?}");
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(read.body["bytes_b64"].as_str().unwrap())
                .unwrap();
            assert_eq!(bytes, b"raw");
            assert_eq!(format!("{:x}", Sha256::digest(&bytes)), files[0]["sha256"]);
        },
    )
    .await;
}

#[tokio::test]
async fn missing_bad_digest_and_escaping_artifacts_fail_the_execution() {
    for path in ["missing.txt", "bad.txt", "../outside.txt"] {
        execute(
            |dir| {
                std::fs::write(dir.join("bad.txt"), b"evil").unwrap();
                write_declaration(dir, path, b"good");
            },
            async |_, detail, _| {
                assert_eq!(detail["execution"]["status"], "error", "{detail}");
                assert!(detail["result"]["scheduler_error"]
                    .as_str()
                    .unwrap()
                    .contains("artifact indexing failed"));
            },
        )
        .await;
    }
}

#[tokio::test]
async fn symlinked_declared_file_is_rejected_before_publishing_a_reference() {
    execute(
        |dir| {
            std::fs::write(dir.join("actual.txt"), b"good").unwrap();
            std::os::unix::fs::symlink("actual.txt", dir.join("report.txt")).unwrap();
            write_declaration(dir, "report.txt", b"good");
        },
        async |_, detail, _| {
            assert_eq!(detail["execution"]["status"], "error", "{detail}");
            assert!(detail["result"]["scheduler_error"]
                .as_str()
                .unwrap()
                .contains("symlink"));
        },
    )
    .await;
}

#[tokio::test]
async fn large_file_index_uses_a_verified_manifest_and_keeps_decision_evidence_bounded() {
    execute(
        |dir| {
            let files: Vec<_> = (0..100)
                .map(|index| {
                    let name = format!("report-{index}-{}.txt", "x".repeat(100));
                    std::fs::write(dir.join(&name), b"good").unwrap();
                    json!({"path":name,"bytes":4,"sha256":format!("{:x}",Sha256::digest(b"good"))})
                })
                .collect();
            std::fs::write(
                dir.join("artifacts.json"),
                json!({"files":files}).to_string(),
            )
            .unwrap();
        },
        async |node, detail, _| {
            assert_eq!(detail["execution"]["status"], "done", "{detail}");
            let files = &detail["result"]["scheduler_artifacts"];
            assert_eq!(files.as_array().unwrap().len(), 1);
            assert_eq!(files[0]["step"], "brain-result");
            assert!(
                serde_json::to_vec(files).unwrap().len()
                    + serde_json::to_vec(&detail["result"]["scheduler_output"])
                        .unwrap()
                        .len()
                    <= 16 * 1024
            );
            let read = node
                .handle(NodeOperation::Artifact {
                    request: request(&files[0]),
                })
                .await;
            assert_eq!(read.status, 200, "{read:?}");
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(read.body["bytes_b64"].as_str().unwrap())
                .unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&bytes)), files[0]["sha256"]);
            let manifest: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(manifest["artifacts"].as_array().unwrap().len(), 100);
            assert!(manifest["artifacts"]
                .as_array()
                .unwrap()
                .iter()
                .all(|file| file["execution"]["id"] == "operator-io-files"));
        },
    )
    .await;
}

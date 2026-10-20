#![cfg(not(windows))]
#[path = "support/mod.rs"]
mod support;

use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

fn copy_image(source: &Path, target: &Path) {
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let copied = std::process::Command::new("cp")
        .args(["-al"])
        .arg(source)
        .arg(target)
        .output()
        .unwrap();
    assert!(copied.status.success(), "{copied:?}");
}

async fn resume(worker: &opencoder_worker::Worker, id: &str) -> RpcReply {
    worker
        .handle(NodeOperation::Command {
            execution: ExecutionRef {
                id: id.into(),
                kind: ExecutionKind::Dag,
            },
            command: ExecutionCommand {
                action: "resume".into(),
                input: json!({}),
            },
        })
        .await
}

#[tokio::test]
async fn retained_dag_uses_explicit_image_upgrade_without_replaying_completed_steps() {
    let (_scope, _home) = support::isolated_config();
    let root = tempfile::tempdir().unwrap();
    let client = support::mock();
    let (native, _bridge) = support::native::environment(root.path(), client.clone());
    let data = root.path().join("node");
    let original = data.join("dag/rootfs");
    let replacement = data.join("programs/upgrade/rootfs");
    copy_image(native.config.dag.rootfs_dir.as_ref().unwrap(), &original);
    copy_image(&original, &replacement);
    let config_file = root.path().join("work/opencoder.json");
    let mut config: Value = serde_json::from_slice(&std::fs::read(&config_file).unwrap()).unwrap();
    config["dag"]["rootfs_dir"] = json!(original);
    std::fs::write(&config_file, serde_json::to_vec(&config).unwrap()).unwrap();
    let node = support::worker(root.path(), client.clone()).await;
    let program = r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
int main(int argc, char **argv) {
  if(argc != 2) return 99;
  if(!strcmp(argv[1],"first")) {
    FILE *count=fopen("/workspace/first-count","a"); fputs("once\n",count); fclose(count);
    FILE *out=fopen("output.json","w"); fputs("{\"saved\":true}",out); fclose(out); return 0;
  }
  if(access("/workspace/retry-intent",F_OK)) {
    FILE *intent=fopen("/workspace/retry-intent","w"); fputs("original",intent); fclose(intent); return 7;
  }
  FILE *out=fopen("output.json","w"); fputs("{\"recovered\":true}",out); fclose(out); return 0;
}
"#;
    support::stage_binary(&data, "image-recovery", program);
    let id = "dag-retained-image";
    let definition = json!({"spec":{"name":"retained-image","steps":[
        {"name":"first","kind":{"type":"binary","resource":"image-recovery","args":["first"]}},
        {"name":"second","depends_on":["first"],"kind":{"type":"binary","resource":"image-recovery","args":["second"]}}
    ]}});
    let accepted = node
        .handle(NodeOperation::Create {
            assignment: support::assignment(
                &node,
                id,
                ExecutionKind::Dag,
                json!({"original":"frozen"}),
                Some(definition),
            ),
        })
        .await;
    assert_eq!(accepted.status, 200, "{accepted:?}");
    let before = support::settled(&node, id).await;
    assert_eq!(before["execution"]["status"], "error", "{before}");
    let run = support::native::run_root(&data, id);
    let record_path = data.join("dag").join(id).join("execution.json");
    let original_record: Value =
        serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    let inputs = std::fs::read(run.join("inputs.json")).unwrap();
    node.shutdown().await.unwrap();
    drop(node);

    // Model the retained image/node version mismatch exposed after maintenance.
    // Remove this private hard link before writing; the shared fixture is untouched.
    let runner = original.join("usr/bin/dag-runner");
    std::fs::remove_file(&runner).unwrap();
    std::fs::write(
        &runner,
        b"#!/bin/sh\nprintf '%s\\n' '{\"git_commit\":\"retained-version\"}'\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&runner, std::fs::Permissions::from_mode(0o755)).unwrap();
    config["dag"]["rootfs_dir"] = json!(replacement);
    std::fs::write(&config_file, serde_json::to_vec(&config).unwrap()).unwrap();
    let reopened = support::worker(root.path(), client).await;
    let rejected = resume(&reopened, id).await;
    assert_eq!(rejected.status, 400, "{rejected:?}");
    assert!(
        rejected.body.to_string().contains("version does not match"),
        "{rejected:?}"
    );
    let hashes: BTreeMap<_, _> = ["dag-runner", "agent-step-runner"]
        .into_iter()
        .map(|name| {
            (
                name,
                format!(
                    "{:x}",
                    Sha256::digest(std::fs::read(replacement.join("usr/bin").join(name)).unwrap())
                ),
            )
        })
        .collect();
    let receipt = json!({"schema":"opencoder.runtime-image-replacement.v1",
        "runtime_data":data.canonicalize().unwrap(),"source_rootfs":[original.canonicalize().unwrap()],
        "target_rootfs":replacement.canonicalize().unwrap(),"runner_sha256":hashes});
    std::fs::write(
        replacement
            .parent()
            .unwrap()
            .join("runtime-image-replacement.json"),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    let resumed = resume(&reopened, id).await;
    assert_eq!(resumed.status, 200, "{resumed:?}");
    let after = support::settled(&reopened, id).await;
    assert_eq!(after["execution"]["status"], "done", "{after}");
    assert_eq!(before["request"], after["request"]);
    assert_eq!(support::native::run_root(&data, id), run);
    assert_eq!(std::fs::read(run.join("inputs.json")).unwrap(), inputs);
    assert_eq!(
        std::fs::read_to_string(run.join("upper/first-count")).unwrap(),
        "once\n"
    );
    let final_record: Value = serde_json::from_slice(&std::fs::read(record_path).unwrap()).unwrap();
    assert_eq!(
        final_record["annotations"]["dag_config"],
        original_record["annotations"]["dag_config"]
    );
    reopened.shutdown().await.unwrap();
}

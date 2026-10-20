#![cfg(not(windows))]
mod support;
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use opencoder_worker::Worker;
use serde_json::{json, Value};
use std::{
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};
use support::*;

const ID: &str = "dag-managed-process-restart";

async fn command(node: &Worker, action: &str, input: Value) -> RpcReply {
    node.handle(NodeOperation::Command {
        execution: ExecutionRef {
            id: ID.into(),
            kind: ExecutionKind::Dag,
        },
        command: ExecutionCommand {
            action: action.into(),
            input,
        },
    })
    .await
}

async fn wait_phase(node: &Worker, phase: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            let reply = command(node, "step-status", json!({})).await;
            if reply.body["steps"].as_array().is_some_and(|rows| {
                rows.iter()
                    .any(|r| r["step"] == "process" && r["phase"] == phase)
            }) {
                return reply.body;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("step did not reach expected phase")
}

#[tokio::test]
async fn process_helper() {
    let Ok(root) = std::env::var("MANAGED_RESTART_FIXTURE") else {
        return;
    };
    let _config = isolated_config();
    let root = Path::new(&root);
    let node = worker(root, mock()).await;
    let spec = json!({"name":"managed-restart","steps":[
        {"name":"completed","recovery_policy":"operator","kind":{"type":"binary","resource":"complete"}},
        {"name":"process","depends_on":["completed"],"recovery_policy":"operator","kind":{"type":"binary","resource":"recover"}}
    ]});
    let accepted = node
        .handle(NodeOperation::Create {
            assignment: assignment(
                &node,
                ID,
                ExecutionKind::Dag,
                json!({"original":true}),
                Some(spec),
            ),
        })
        .await;
    assert_eq!(accepted.status, 200, "{accepted:?}");
    wait_phase(&node, "running").await;
    let started = dag_run(&root.join("node"), ID).join("workspace/process/started");
    while !started.exists() {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    std::fs::write(root.join("ready"), node.registration().id).unwrap();
    std::future::pending::<()>().await;
}

#[tokio::test]
async fn killed_worker_resumes_original_dag_without_replaying_completed_step() {
    let _config = isolated_config();
    let dir = tempfile::tempdir().unwrap();
    // The fixture's NFS and model bridge survive the worker process, just as
    // independent platform services do during a real node restart.
    let (_native, _bridge) = support::native::environment(dir.path(), mock());
    stage_stdout_binary(
        &dir.path().join("node"),
        "complete",
        "immutable completed output",
    );
    stage_binary(
        &dir.path().join("node"),
        "recover",
        r#"
#include <fcntl.h>
#include <unistd.h>
int main(void){
 int log=open("attempts",O_CREAT|O_APPEND|O_WRONLY,0600);write(log,"x",1);close(log);
 int first=open("started",O_CREAT|O_EXCL|O_WRONLY,0600);
 if(first>=0){close(first);for(;;)pause();}
 return 0;
}
"#,
    );
    let mut process = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "process_helper", "--nocapture"])
        .env("MANAGED_RESTART_FIXTURE", dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let ready = tokio::time::timeout(Duration::from_secs(45), async {
        while !dir.path().join("ready").exists() {
            assert!(
                process.try_wait().unwrap().is_none(),
                "worker exited before crash boundary"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    process.kill().unwrap();
    process.wait().unwrap();
    ready.expect("worker did not reach crash boundary");
    let run = dag_run(&dir.path().join("node"), ID);
    let completed = std::fs::read(run.join("completed/meta.json")).unwrap();
    let first_node = std::fs::read_to_string(dir.path().join("ready")).unwrap();
    let node = worker(dir.path(), mock()).await;
    assert_eq!(node.registration().id, first_node);
    assert_eq!(command(&node, "resume", json!({})).await.status, 200);
    let held = wait_phase(&node, "waiting_recovery").await;
    assert!(held["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["step"] == "completed" && r["phase"] == "done" && r["attempt"] == 1));
    assert_eq!(
        std::fs::read(run.join("upper/process/attempts")).unwrap(),
        b"x"
    );
    let resumed = command(
        &node,
        "step-resume",
        json!({"step":"process","expected_attempt":1,
        "operation_id":"continue-original","reason":"verified original worker process loss"}),
    )
    .await;
    assert_eq!(resumed.status, 200, "{resumed:?}");
    assert_eq!(settled(&node, ID).await["execution"]["status"], "done");
    assert_eq!(
        std::fs::read(run.join("completed/meta.json")).unwrap(),
        completed
    );
    assert_eq!(
        std::fs::read(run.join("upper/process/attempts")).unwrap(),
        b"xx"
    );
    assert!(run.join("process/attempts/1/context.json").exists());
    node.shutdown().await.unwrap();
}

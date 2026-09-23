//! Milestone wake delivery. A wake may only acknowledge the generation its
//! own activation admitted: a newer Ready generation that appears while the
//! delivery is in flight gets its own wake and its own acknowledgement.
use crate::transport::SocketCommand;
use opencoder_core::fleet::*;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const NODE: &str = "layered-node";
const CONNECTION: &str = "layered";

/// Live node + root index; the directory keeps the control DB alive.
async fn boot(
    id: &str,
) -> (
    tempfile::TempDir,
    Arc<crate::AppState>,
    tokio::sync::mpsc::Receiver<SocketCommand>,
) {
    let directory = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(directory.path().join("config"));
    let state = crate::new_state(
        directory.path().join("work"),
        directory.path().join("data"),
        None,
    )
    .await
    .unwrap();
    state
        .fleet
        .put_index(&ExecutionIndex {
            id: id.into(),
            node_id: NODE.into(),
            kind: ExecutionKind::Brain,
            status: ExecutionStatus::Idle,
            created_at: 1,
        })
        .await
        .unwrap();
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    state
        .hub
        .attach(
            NodeRegistration {
                id: NODE.into(),
                name: "layered test".into(),
                version: "test".into(),
                protocol_version: PROTOCOL_VERSION,
                maintenance_agent_id: "maintenance-layered-node".into(),
                kinds: vec![ExecutionKind::Brain],
            },
            NodeSnapshot {
                generation: CONNECTION.into(),
                sequence: 1,
                cpu_capacity: 1.0,
                pending_runs: 0,
                active_agent_loops: 0,
                active_runs: 0,
                max_runs: 4,
                queue_order: Default::default(),
                ready: true,
                resource_error: None,
            },
            tx,
        )
        .await
        .unwrap();
    assert!(state.hub.mark_index_synced(NODE, CONNECTION).await);
    (directory, state, rx)
}

fn root(id: &str) -> ExecutionRef {
    ExecutionRef {
        id: id.into(),
        kind: ExecutionKind::Brain,
    }
}

/// One-node plan bound to the built-in agent capability, so no catalog seed
/// is needed and layer 1 is always dispatchable.
fn layered_request() -> Value {
    json!({"schema_version":5,
        "plan":{"schema_version":5,"title":"layered wake","objective":"admit one generation",
            "nodes":[{"node_id":"scan","title":"Scan","layer":1,
                "objective":"inspect", "success_criteria":"evidence read",
                "capability_ids":["builtin-agent-act"]}],
            "edges":[],"max_rounds":8},
        "inputs":{},"artifacts":{},"depth":0})
}

/// A layered snapshot the node could own: `phase`, `generation` and `layer`
/// are the only fields these tests vary.
fn snapshot(id: &str, phase: &str, generation: u64, layer: u32) -> Value {
    json!({"schema_version":5,
        "run":{"run_id":id,"phase":phase,"layer":layer,"generation":generation,
            "last_event_seq":0,"error":null,"created_at":1,"updated_at":2},
        "operations":[]})
}

async fn attach_assignment(state: &Arc<crate::AppState>, id: &str) {
    let assignment = Assignment {
        private_context: None,
        runtime: None,
        codex: None,
        definition: None,
        index: ExecutionIndex {
            id: id.into(),
            node_id: NODE.into(),
            kind: ExecutionKind::Brain,
            status: ExecutionStatus::Idle,
            created_at: 1,
        },
        request: CreateExecution {
            id: id.into(),
            kind: ExecutionKind::Brain,
            target: None,
            input: json!({"schema_version":5,"layered_request":layered_request(),
                "frozen_capabilities":[{"capability_id":"builtin-agent-act","kind":"agent",
                "target":"act","input_desc":"input","output_desc":"output","required_inputs":[],
                "definition":{"name":"act"},"version":"1"}]}),
            node_id: None,
        },
    };
    let fingerprint =
        opencoder_core::token_hash(&serde_json::to_string(&assignment.request).unwrap());
    assert!(state
        .fleet
        .claim_request("execution", id, &fingerprint)
        .await
        .unwrap());
    state
        .fleet
        .prepare_assignment(&assignment, &fingerprint)
        .await
        .unwrap();
}

#[tokio::test]
async fn stale_layered_wake_does_not_acknowledge_a_new_ready_generation() {
    let (_directory, state, mut rx) = boot("brain-layered-stale").await;
    let acknowledgements = Arc::new(Mutex::new(Vec::new()));
    let received = acknowledgements.clone();
    let owner = state.clone();
    let node = tokio::spawn(async move {
        let mut reads = 0;
        loop {
            let Some(SocketCommand::Frame(frame)) = rx.recv().await else {
                return;
            };
            let ServerFrame::Call {
                request_id,
                operation,
            } = *frame;
            let NodeOperation::Brain { action, input, .. } = operation else {
                panic!("expected a brain RPC");
            };
            let body = match action.as_str() {
                "snapshot" => {
                    reads += 1;
                    // The activation committed while this wake was in flight:
                    // a second read would already see the next ready round.
                    if reads == 1 {
                        snapshot("brain-layered-stale", "deciding", 1, 0)
                    } else {
                        snapshot("brain-layered-stale", "ready", 4, 1)
                    }
                }
                "layered_wake_ack" => {
                    received
                        .lock()
                        .unwrap()
                        .push(input["generation"].as_u64().unwrap());
                    json!({"acknowledged":input["generation"]})
                }
                other => panic!("unexpected layered action {other}"),
            };
            owner
                .hub
                .resolve(NODE, CONNECTION, &request_id, RpcReply::ok(body))
                .await;
            if action == "layered_wake_ack" {
                return;
            }
        }
    });
    crate::api::brain_runs::v4::delivery::deliver(
        &state,
        NODE,
        &root("brain-layered-stale"),
        "layered_wake",
        json!({"generation":1}),
    )
    .await
    .unwrap();
    assert_eq!(
        *acknowledgements.lock().unwrap(),
        vec![1],
        "a stale wake must not consume the next ready generation"
    );
    node.abort();
    let _ = node.await;
}

#[tokio::test]
async fn layered_wake_acknowledges_the_generation_its_activation_admitted() {
    let (_directory, state, mut rx) = boot("brain-layered-ack").await;
    attach_assignment(&state, "brain-layered-ack").await;
    let acknowledgements = Arc::new(Mutex::new(Vec::new()));
    let received = acknowledgements.clone();
    let owner = state.clone();
    let node = tokio::spawn(async move {
        loop {
            let Some(SocketCommand::Frame(frame)) = rx.recv().await else {
                return;
            };
            let ServerFrame::Call {
                request_id,
                operation,
            } = *frame;
            let NodeOperation::Brain { action, input, .. } = operation else {
                panic!("expected a brain RPC");
            };
            let body = match action.as_str() {
                "snapshot" => snapshot("brain-layered-ack", "ready", 2, 0),
                "layered_context" => {
                    // The node admits the activation and publishes the next
                    // generation; control acknowledges the generation it saw.
                    assert_eq!(
                        input["layer"],
                        json!(0),
                        "context keeps the admitted layer watermark"
                    );
                    assert_eq!(
                        input["request"]["plan"]["nodes"][0]["node_id"],
                        json!("scan")
                    );
                    snapshot("brain-layered-ack", "deciding", 3, 0)
                }
                "layered_wake_ack" => {
                    received
                        .lock()
                        .unwrap()
                        .push(input["generation"].as_u64().unwrap());
                    json!({"acknowledged":input["generation"]})
                }
                other => panic!("unexpected layered action {other}"),
            };
            owner
                .hub
                .resolve(NODE, CONNECTION, &request_id, RpcReply::ok(body))
                .await;
            if action == "layered_wake_ack" {
                return;
            }
        }
    });
    crate::api::brain_runs::v4::delivery::deliver(
        &state,
        NODE,
        &root("brain-layered-ack"),
        "layered_wake",
        json!({"generation":1}),
    )
    .await
    .unwrap();
    assert_eq!(*acknowledgements.lock().unwrap(), vec![3]);
    node.abort();
    let _ = node.await;
}

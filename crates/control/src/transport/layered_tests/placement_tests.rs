//! Exercise gateway dispatch through the real scheduler and frozen assignments.
use crate::transport::SocketCommand;
use crate::{api::brain_runs::v4::delivery, AppState};
use opencoder_core::{brain::layered::*, brain::*, fleet::*};
use serde_json::{json, Value};
use std::sync::Arc;

async fn node(
    state: &Arc<AppState>,
    id: &str,
    loops: u64,
    snapshot: Value,
) -> tokio::task::JoinHandle<()> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    state
        .hub
        .attach(
            NodeRegistration {
                id: id.into(),
                name: id.into(),
                version: "test".into(),
                protocol_version: PROTOCOL_VERSION,
                maintenance_agent_id: "act".into(),
                kinds: vec![ExecutionKind::Brain, ExecutionKind::Dag],
            },
            NodeSnapshot {
                generation: id.into(),
                sequence: 1,
                cpu_capacity: 4.0,
                pending_runs: 0,
                active_agent_loops: loops,
                active_runs: loops,
                max_runs: 4,
                queue_order: Default::default(),
                ready: true,
                resource_error: None,
            },
            tx,
        )
        .await
        .unwrap();
    assert!(state.hub.mark_index_synced(id, id).await);
    let state = state.clone();
    let id = id.to_owned();
    tokio::spawn(async move {
        while let Some(SocketCommand::Frame(frame)) = rx.recv().await {
            let ServerFrame::Call {
                request_id,
                operation,
            } = *frame;
            let reply = match operation {
                NodeOperation::Brain { action, .. } if action == "capability_probe" => {
                    RpcReply::ok(json!({"compatible":true,"features":[
                        "dag_container_v1","brain_scheduler_v7","brain_contracts_v1"
                    ]}))
                }
                NodeOperation::Brain { action, input, .. } => match action.as_str() {
                    "snapshot" => RpcReply::ok(snapshot.clone()),
                    "layered_receipt" => {
                        assert!(input["reply"]["status"].as_u64().unwrap() < 300, "{input}");
                        RpcReply::ok(json!({}))
                    }
                    "layered_authorize" | "layered_dispatch_ack" => RpcReply::ok(json!({})),
                    _ => panic!("unexpected Brain action: {action}"),
                },
                NodeOperation::Create { assignment } => {
                    assert_eq!(assignment.index.node_id, id);
                    RpcReply::ok(json!(assignment.index))
                }
                _ => panic!("unexpected placement RPC"),
            };
            state.hub.resolve(&id, &id, &request_id, reply).await;
        }
    })
}

async fn dispatch_child(explicit_node: Option<&str>, expected: &str) {
    let directory = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(directory.path().join("config"));
    let state = crate::new_state(
        directory.path().join("work"),
        directory.path().join("data"),
        None,
    )
    .await
    .unwrap();
    // B owns the root; A is idle and would win an unconstrained child dispatch.
    let root_id = "brain-placement";
    let root = Assignment {
        private_context: None,
        runtime: None,
        codex: None,
        definition: None,
        index: ExecutionIndex {
            id: root_id.into(),
            node_id: "physical-b".into(),
            kind: ExecutionKind::Brain,
            status: ExecutionStatus::Idle,
            created_at: 1,
        },
        request: CreateExecution {
            id: root_id.into(),
            kind: ExecutionKind::Brain,
            target: None,
            node_id: explicit_node.map(str::to_owned),
            input: json!({"layered_request":{"inputs":{},"plan":{
                "schema_version":7,"title":"placement","objective":"retain caller intent",
                "nodes":[{"node_id":"logical-step","title":"Inspect","layer_id":"inspect",
                    "objective":"inspect","capability_id":"dag-inspect"}],
                "layers":[{"layer_id":"inspect","title":"Inspect","task":"inspect",
                    "objective":"inspect","success_criteria":"evidence"}],
                "edges":[],"max_rounds":3
            }}}),
        },
    };
    let fingerprint = opencoder_core::token_hash(&serde_json::to_string(&root.request).unwrap());
    assert!(state
        .fleet
        .claim_request("execution", root_id, &fingerprint)
        .await
        .unwrap());
    state
        .fleet
        .prepare_assignment(&root, &fingerprint)
        .await
        .unwrap();
    let run: LayeredRun = serde_json::from_value(json!({
        "run_id":root_id,"phase":"waiting","layer":0,"generation":1,
        "last_event_seq":0,"error":null,"created_at":1,"updated_at":1
    }))
    .unwrap();
    let operation: LayeredOperation = serde_json::from_value(json!({
        "operation_id":"op-inspect","run_id":root_id,"layer":1,"node_id":"logical-step",
        "attempt":1,"capability_id":"dag-inspect","execution_kind":"dag",
        "execution_id":"dag-placement-child","status":"creating",
        "source_sequence":null,"cancel_requested":false
    }))
    .unwrap();
    let assignment: LayeredAssignment = serde_json::from_value(json!({
        "node_id":"logical-step","capability_id":"dag-inspect","inputs":{},"reason":"inspect"
    }))
    .unwrap();
    let cap: BrainCapabilityDescriptor = serde_json::from_value(json!({
        "capability_id":"dag-inspect","kind":"dag","target":"inspect",
        "input_desc":"input","output_desc":"evidence","version":"1",
        "definition":{"name":"inspect","steps":[{"name":"probe","kind":{
            "type":"binary","resource":"inspect@v1"}}]}
    }))
    .unwrap();
    let snapshot = json!({"schema_version":7,"run":run,"operations":[operation]});
    let a = node(&state, "physical-a", 0, snapshot.clone()).await;
    let b = node(&state, "physical-b", 1, snapshot).await;
    let source = ExecutionRef {
        id: root_id.into(),
        kind: ExecutionKind::Brain,
    };
    let payload = json!({"operation":operation,"assignment":assignment,"capability":cap});
    delivery::deliver(
        &state,
        "physical-b",
        &source,
        "layered_dispatch",
        payload.clone(),
    )
    .await
    .unwrap();
    let child = state
        .fleet
        .assignment(&operation.execution_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(child.index.node_id, expected);
    assert_eq!(child.request.node_id.as_deref(), explicit_node);
    assert_eq!(
        child.request.input["brain_layered"]["node_id"],
        "logical-step"
    );
    // Replaying delivery preserves the same request, selected node and receipt.
    delivery::deliver(&state, "physical-b", &source, "layered_dispatch", payload)
        .await
        .unwrap();
    assert_eq!(
        json!(state
            .fleet
            .assignment(&operation.execution_id)
            .await
            .unwrap()
            .unwrap()),
        json!(child)
    );
    assert_eq!(
        json!(state.fleet.assignment(root_id).await.unwrap().unwrap()),
        json!(root)
    );
    a.abort();
    b.abort();
    let _ = tokio::join!(a, b);
}

#[tokio::test]
async fn explicit_physical_node_is_retained_by_child_dispatch() {
    dispatch_child(Some("physical-b"), "physical-b").await;
}

#[tokio::test]
async fn unspecified_physical_node_keeps_independent_child_scheduling() {
    dispatch_child(None, "physical-a").await;
}

#[path = "scheduler_v4/client.rs"]
mod client;
mod support;

use client::LayeredClient;
use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, RequestPurpose};
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{mpsc, Semaphore};

struct ControlledClient {
    layered: LayeredClient,
    planning_calls: AtomicUsize,
    child_calls: AtomicUsize,
    hold_first_planning: bool,
    hold_first_child: bool,
    planning_gate: Arc<Semaphore>,
    child_gate: Arc<Semaphore>,
}

impl ControlledClient {
    fn new(hold_first_planning: bool, hold_first_child: bool) -> Arc<Self> {
        Arc::new(Self {
            layered: LayeredClient::new(),
            planning_calls: AtomicUsize::new(0),
            child_calls: AtomicUsize::new(0),
            hold_first_planning,
            hold_first_child,
            planning_gate: Arc::new(Semaphore::new(0)),
            child_gate: Arc::new(Semaphore::new(0)),
        })
    }
}

impl ChatStream for ControlledClient {
    fn chat_stream(&self, request: ChatRequest) -> anyhow::Result<mpsc::Receiver<LlmEvent>> {
        let planning = request.purpose == RequestPurpose::Planning;
        let ordinal = if planning {
            self.planning_calls.fetch_add(1, Ordering::SeqCst)
        } else {
            self.child_calls.fetch_add(1, Ordering::SeqCst)
        };
        let mut upstream = self.layered.chat_stream(request)?;
        let gate = if ordinal == 0 && self.hold_first_planning && planning {
            Some(self.planning_gate.clone())
        } else if ordinal == 0 && self.hold_first_child && !planning {
            Some(self.child_gate.clone())
        } else {
            None
        };
        if let Some(gate) = gate {
            let (sender, receiver) = mpsc::channel(1);
            tokio::spawn(async move {
                let permit = gate.acquire().await.expect("test gate closed");
                permit.forget();
                while let Some(event) = upstream.recv().await {
                    if sender.send(event).await.is_err() {
                        break;
                    }
                }
            });
            Ok(receiver)
        } else {
            Ok(upstream)
        }
    }
}

fn plan(parallel: bool) -> Value {
    let mut nodes = vec![json!({
        "node_id":"coding", "title":"Code", "objective":"make the change",
        "capability_id":"builtin-agent-act", "layer_id":"work"
    })];
    if parallel {
        nodes.push(json!({
            "node_id":"review", "title":"Review", "objective":"review the change",
            "capability_id":"builtin-agent-act", "layer_id":"work"
        }));
    }
    json!({
        "schema_version":7, "title":"Restart recovery", "objective":"finish the work",
        "nodes":nodes,
        "layers":[{"layer_id":"work","title":"Work","objective":"finish the work","success_criteria":"both executions completed"}],
        "transitions":[], "edges":[]
    })
}

async fn until(mut predicate: impl AsyncFnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !predicate().await {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("restart acceptance condition timed out");
}

async fn completed(fleet: &support::Fleet, id: &str, expected: usize) -> Vec<Value> {
    let mut result = Vec::new();
    until(async || {
        let reply = fleet
            .call("GET", &format!("/api/brain/runs/{id}/layered"), Value::Null)
            .await;
        assert_eq!(reply.status, 200, "{reply:?}");
        if reply.body["run"]["phase"] == "completed" {
            result = reply.body["operations"].as_array().unwrap().clone();
            true
        } else {
            assert_ne!(reply.body["run"]["phase"], "blocked", "{reply:?}");
            false
        }
    })
    .await;
    assert_eq!(result.len(), expected, "{result:?}");
    let ids: HashSet<_> = result
        .iter()
        .map(|op| op["execution_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), expected, "duplicate child execution IDs");
    for operation in &result {
        assert_eq!(operation["status"], "done", "{operation:?}");
        let id = operation["execution_id"].as_str().unwrap();
        let detail = fleet
            .call("GET", &format!("/api/executions/{id}"), Value::Null)
            .await;
        assert_eq!(
            detail.status, 200,
            "missing child detail for {id}: {detail:?}"
        );
        assert_eq!(detail.body["execution"]["status"], "done");
    }
    let owned: Vec<_> = fleet.nodes[0]
        .indexes()
        .await
        .unwrap()
        .into_iter()
        .filter(|index| index.kind == opencoder_core::fleet::ExecutionKind::Agent)
        .collect();
    assert_eq!(
        owned.len(),
        expected,
        "unexpected child executions: {owned:?}"
    );
    let owned_ids: HashSet<_> = owned.iter().map(|index| index.id.as_str()).collect();
    assert_eq!(owned_ids, ids, "child IDs differ from the brain projection");
    result
}

#[tokio::test]
async fn server_restart_preserves_running_parallel_children_and_resumes_brain() {
    let model = ControlledClient::new(false, true);
    let mut fleet = support::Fleet::new(1, model.clone()).await;
    let root = "brain-restart-running";
    let created = fleet
        .call(
            "POST",
            "/api/brain/runs",
            json!({"id":root,"schema_version":7,"plan":plan(true)}),
        )
        .await;
    assert_eq!(created.status, 202, "{created:?}");
    until(async || model.child_calls.load(Ordering::SeqCst) >= 2).await;
    let before = fleet
        .call(
            "GET",
            &format!("/api/brain/runs/{root}/layered"),
            Value::Null,
        )
        .await;
    assert_eq!(before.body["run"]["phase"], "waiting", "{before:?}");
    assert_eq!(before.body["run"]["layer"], 1);
    let before_ids: HashSet<_> = before.body["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| op["execution_id"].as_str().unwrap().to_string())
        .collect();
    let running_id = before.body["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["status"] != "done")
        .unwrap()["execution_id"]
        .as_str()
        .unwrap()
        .to_string();
    fleet.stop_server().await;
    let running = fleet.nodes[0]
        .indexes()
        .await
        .unwrap()
        .into_iter()
        .find(|index| index.id == running_id)
        .unwrap();
    assert_eq!(
        running.status,
        opencoder_core::fleet::ExecutionStatus::Running
    );
    model.child_gate.add_permits(1);
    until(async || {
        fleet.nodes[0].indexes().await.unwrap().iter().any(|index| {
            index.id == running_id && index.status == opencoder_core::fleet::ExecutionStatus::Done
        })
    })
    .await;
    fleet.restart_server().await;
    let operations = completed(&fleet, root, 2).await;
    let after_ids: HashSet<_> = operations
        .iter()
        .map(|op| op["execution_id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(before_ids, after_ids, "recovery replaced an execution ID");
    assert!(operations.iter().any(|op| op["execution_id"] == running_id));
    fleet.shutdown().await;
}

#[tokio::test]
async fn server_restart_replays_a_decision_committed_while_offline() {
    let model = ControlledClient::new(true, false);
    let mut fleet = support::Fleet::new(1, model.clone()).await;
    let root = "brain-restart-undelivered";
    let created = fleet
        .call(
            "POST",
            "/api/brain/runs",
            json!({"id":root,"schema_version":7,"plan":plan(false)}),
        )
        .await;
    assert_eq!(created.status, 202, "{created:?}");
    until(async || model.planning_calls.load(Ordering::SeqCst) >= 1).await;
    fleet.stop_server().await;
    model.planning_gate.add_permits(1);
    until(async || {
        fleet.nodes[0].brain_frames().await.unwrap().iter().any(|frame| {
            matches!(frame, opencoder_core::fleet::NodeFrame::Brain { action, .. } if action == "layered_dispatch")
        })
    })
    .await;
    fleet.restart_server().await;
    completed(&fleet, root, 1).await;
    fleet.shutdown().await;
}

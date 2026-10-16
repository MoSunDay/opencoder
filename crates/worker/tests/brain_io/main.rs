#![cfg(not(windows))]
mod adapters;
mod artifacts;
mod context;
mod fixture;
mod model;
mod recovery;
#[path = "../support/mod.rs"]
mod support;

use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::{json, Value};
use std::{collections::HashSet, sync::Arc, time::Duration};
use support::Fleet;

async fn completed(fleet: &Fleet, id: &str) -> Value {
    let mut last = Value::Null;
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            let reply = fleet
                .call("GET", &format!("/api/brain/runs/{id}/layered"), Value::Null)
                .await;
            last = json!({"run":reply.body["run"],"operations":reply.body["operations"]});
            assert_eq!(reply.status, 200, "{reply:?}");
            assert!(
                !matches!(
                    reply.body["run"]["phase"].as_str(),
                    Some("failed" | "blocked")
                ),
                "run {id}: {}",
                reply.body["run"]["error"]
            );
            if reply.body["run"]["phase"] == "completed" {
                return reply.body;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|error| panic!("IO run {id} did not complete: {error}; {last}"))
}

async fn submit(fleet: &Fleet, id: &str, seed: &str, plan: Value) {
    let reply = fleet
        .call(
            "POST",
            "/api/brain/runs",
            json!({"id":id,"schema_version":7,"plan":plan,"inputs":{"seed":seed}}),
        )
        .await;
    assert_eq!(reply.status, 202, "{reply:?}");
}

#[tokio::test]
async fn named_inputs_dynamic_artifacts_cross_nodes_and_server_restart_preserve_sources() {
    let model = Arc::new(model::Model::new(true));
    let mut fleet = Fleet::new(2, model.clone()).await;
    *model.url.lock().unwrap() = fleet.url.clone();
    fixture::prepare(&fleet).await;
    submit(&fleet, "brain-io-cross", "cross", fixture::plan()).await;
    tokio::time::timeout(Duration::from_secs(60), async {
        while !model
            .contexts
            .lock()
            .unwrap()
            .iter()
            .any(|c| c["run"]["layer"] == 2)
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let before = fleet
        .call("GET", "/api/brain/runs/brain-io-cross/layered", Value::Null)
        .await
        .body;
    let dag = before["operations"][1]["execution_id"].as_str().unwrap();
    let source = fleet.state.fleet.index(dag).await.unwrap().unwrap();
    fleet.stop_server().await;
    fleet.restart_server().await;
    let node = fleet
        .nodes
        .iter()
        .find(|n| n.registration().id == source.node_id)
        .unwrap();
    for id in ["operator-io-source-load-a", "operator-io-source-load-b"] {
        let reply = node
            .handle(NodeOperation::Create {
                assignment: support::assignment(
                    node,
                    id,
                    ExecutionKind::Operator,
                    json!({"prompt":"Keep the IO source node busy"}),
                    None,
                ),
            })
            .await;
        assert_eq!(reply.status, 200, "{reply:?}");
    }
    let configured = node
        .handle(NodeOperation::Maintenance {
            command: ExecutionCommand {
                action: "configure_scheduling".into(),
                input: json!({"max_runs":2}),
            },
        })
        .await;
    assert_eq!(configured.status, 200, "{configured:?}");
    // Occupy the source's real execution slots. These fixtures share a process
    // and its loop registry, so CPU-loop counts cannot distinguish their nodes.
    tokio::time::timeout(Duration::from_secs(15), async {
        while !fleet.state.hub.views().await.iter().any(|node| {
            node.registration.id == source.node_id
                && node
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.max_runs == 2 && snapshot.active_runs >= 2)
        }) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("source-node work did not reach the scheduler");
    model.gate.add_permits(1);
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let view = fleet
                .call("GET", "/api/brain/runs/brain-io-cross/layered", Value::Null)
                .await;
            if let Some(id) = view.body["operations"][2]["execution_id"].as_str() {
                if let Some(index) = fleet.state.fleet.index(id).await.unwrap() {
                    assert_ne!(index.node_id, source.node_id);
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("consumer did not acquire its own execution node");
    model.load_gate.add_permits(2);
    let view = completed(&fleet, "brain-io-cross").await;
    assert_eq!(view["operations"].as_array().unwrap().len(), 3);
    assert_eq!(
        view["operations"][1]["execution_id"],
        before["operations"][1]["execution_id"]
    );
    let consumer = view["operations"][2]["execution_id"].as_str().unwrap();
    let destination = fleet.state.fleet.index(consumer).await.unwrap().unwrap();
    assert_ne!(source.node_id, destination.node_id);
    let detail = fleet
        .call("GET", &format!("/api/executions/{consumer}"), Value::Null)
        .await
        .body;
    assert_eq!(detail["result"]["scheduler_output"]["origin"], dag);
    assert_eq!(detail["result"]["scheduler_output"]["file_count"], 3);
    let root = fleet
        .call("GET", "/api/executions/brain-io-cross", Value::Null)
        .await
        .body;
    assert_eq!(
        model::verify_files(&fleet.url, &root["result"]["scheduler_artifacts"], "cross").await,
        3
    );
    fleet.shutdown().await;
}

#[tokio::test]
async fn concurrent_runs_keep_same_capability_inputs_outputs_and_files_separate() {
    let model = Arc::new(model::Model::new(false));
    let fleet = Fleet::new(2, model.clone()).await;
    *model.url.lock().unwrap() = fleet.url.clone();
    fixture::prepare(&fleet).await;
    submit(&fleet, "brain-io-a", "A", fixture::plan()).await;
    submit(&fleet, "brain-io-b", "B", fixture::plan()).await;
    let a = completed(&fleet, "brain-io-a").await;
    let b = completed(&fleet, "brain-io-b").await;
    let ids: HashSet<_> = a["operations"]
        .as_array()
        .unwrap()
        .iter()
        .chain(b["operations"].as_array().unwrap())
        .map(|op| op["execution_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 6);
    for (view, seed) in [(&a, "A"), (&b, "B")] {
        let id = view["operations"][2]["execution_id"].as_str().unwrap();
        let detail = fleet
            .call("GET", &format!("/api/executions/{id}"), Value::Null)
            .await
            .body;
        assert_eq!(detail["request"]["input"]["layered_inputs"]["seed"], seed);
        assert_eq!(
            detail["result"]["scheduler_output"]["revision"],
            format!("rev-{seed}")
        );
    }
    fleet.shutdown().await;
}

#[tokio::test]
async fn nested_brain_exports_a_verified_manifest_to_the_parent_consumer() {
    let model = Arc::new(model::Model::new(false));
    let fleet = Fleet::new(2, model.clone()).await;
    *model.url.lock().unwrap() = fleet.url.clone();
    fixture::prepare(&fleet).await;
    let saved = fleet
        .call(
            "POST",
            "/api/brain/plan-defs",
            json!({"id":"io-child","version":1,
        "plan":fixture::plan(),"changelog":"IO fixture","created_at":1}),
        )
        .await;
    assert_eq!(saved.status, 200, "{saved:?}");
    submit(&fleet, "brain-io-parent", "nested", fixture::wrapper()).await;
    let view = completed(&fleet, "brain-io-parent").await;
    assert_eq!(view["operations"][0]["execution_kind"], "brain");
    let child = view["operations"][0]["execution_id"].as_str().unwrap();
    let detail = fleet
        .call("GET", &format!("/api/executions/{child}"), Value::Null)
        .await
        .body;
    assert_eq!(
        detail["result"]["scheduler_artifacts"][0]["step"],
        "brain-result"
    );
    assert_eq!(
        model::verify_files(
            &fleet.url,
            &detail["result"]["scheduler_artifacts"],
            "nested"
        )
        .await,
        3
    );
    fleet.shutdown().await;
}

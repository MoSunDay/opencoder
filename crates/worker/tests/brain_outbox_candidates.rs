#![cfg(not(windows))]
//! Real journal loading, bounded replay, persisted acknowledgements and restart.
mod support;
use opencoder_core::fleet::*;
use opencoder_node::fleet::NodeService;
use serde_json::json;
use std::collections::BTreeSet;
use support::*;

#[tokio::test]
async fn terminal_candidates_rotate_past_sixty_four_and_acks_survive_restart() {
    let (_scope, _home) = isolated_config();
    let root = tempfile::tempdir().unwrap();
    let layout = opencoder_worker::DirectoryLayout::new(root.path().join("node"), None).unwrap();
    for n in 0..130 {
        let id = format!("agent-replay-{n:03}");
        let path = layout.record_path(ExecutionKind::Agent, &id).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let record = json!({"assignment":{
            "index":{"id":id,"kind":"agent","node_id":"node-replay","created_at":n,"status":"done"},
            "request":{"id":id,"kind":"agent","input":{"schema_version":7,
                "brain_layered":{"parent":{"run_id":"brain-parent","operation_id":format!("op-{n}"),"node_id":"step","layer":1}}}}},
            "result":null,"error":null,"events":[{"kind":"done","data":{},"ts":1,"seq":1}]});
        std::fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
    }
    let node = worker(root.path(), mock()).await;
    let record_path = layout
        .record_path(ExecutionKind::Agent, "agent-replay-000")
        .unwrap();
    let original = std::fs::read(&record_path).unwrap();
    let mut seen = BTreeSet::new();
    let mut acknowledged = BTreeSet::new();
    for round in 0..3 {
        let frames = node.brain_frames().await.unwrap();
        assert_eq!(frames.len(), 64);
        for frame in frames {
            let NodeFrame::Brain {
                execution, action, ..
            } = frame
            else {
                panic!()
            };
            assert_eq!(action, "layered_terminal");
            seen.insert(execution.id.clone());
            if round == 0 {
                acknowledged.insert(execution.id);
            }
        }
    }
    assert_eq!(
        seen.len(),
        130,
        "unacknowledged first page cannot starve later records"
    );
    let writer = opencoder_core::fleet::release::ReportStateCompatibility::current().writer;
    if writer == 1 {
        std::fs::write(&record_path, b"invalid JSON must not be read by ACK").unwrap();
    }
    for id in &acknowledged {
        let reply = node
            .handle(NodeOperation::Brain {
                execution: ExecutionRef {
                    id: id.clone(),
                    kind: ExecutionKind::Agent,
                },
                action: "notice_ack".into(),
                input: json!({"sequence":1}),
            })
            .await;
        assert_eq!(reply.status, 200, "{reply:?}");
    }
    if writer == 1 {
        assert_eq!(
            std::fs::read(&record_path).unwrap(),
            b"invalid JSON must not be read by ACK",
            "writer ACK must not read or rewrite the complete execution file"
        );
        std::fs::write(&record_path, &original).unwrap();
    }
    node.shutdown().await.unwrap();
    drop(node);
    let reopened = worker(root.path(), mock()).await;
    let mut remaining = BTreeSet::new();
    for _ in 0..2 {
        for frame in reopened.brain_frames().await.unwrap() {
            let NodeFrame::Brain { execution, .. } = frame else {
                panic!()
            };
            assert!(!acknowledged.contains(&execution.id));
            remaining.insert(execution.id);
        }
    }
    assert_eq!(remaining.len(), 66);
    reopened.shutdown().await.unwrap();
}

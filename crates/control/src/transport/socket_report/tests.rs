use super::*;
use crate::transport::SocketCommand;
use futures::StreamExt;
use opencoder_core::fleet::*;
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;

async fn attach(
    state: &AppState,
    generation: &str,
) -> (mpsc::Sender<SocketCommand>, mpsc::Receiver<SocketCommand>) {
    let (tx, rx) = mpsc::channel(128);
    state
        .hub
        .attach(
            NodeRegistration {
                id: "node-report-progress".into(),
                name: "report progress".into(),
                version: "test".into(),
                protocol_version: PROTOCOL_VERSION,
                maintenance_agent_id: "maintenance-report-progress".into(),
                kinds: vec![ExecutionKind::Agent],
            },
            NodeSnapshot {
                generation: generation.into(),
                sequence: 1,
                cpu_capacity: 4.0,
                active_agent_loops: 0,
                active_runs: 0,
                pending_runs: 0,
                max_runs: 4,
                queue_order: Default::default(),
                ready: true,
                resource_error: None,
            },
            tx.clone(),
        )
        .await
        .unwrap();
    (tx, rx)
}

fn report(initial: bool) -> CompleteReport {
    CompleteReport {
        report_id: 1,
        records: vec![],
        pending_at_begin: Some(vec![]),
        initial,
    }
}

#[tokio::test]
async fn full_rpc_queue_does_not_block_initial_host_handoff() {
    let root = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(root.path().join("config"));
    let state = crate::new_state(root.path().join("work"), root.path().join("data"), None)
        .await
        .unwrap();
    let generation = "host-00000000000000000001-progress";
    let (tx, mut rx) = attach(&state, generation).await;
    assert!(
        state
            .hub
            .mark_index_synced("node-report-progress", generation)
            .await
    );
    let mut calls = tokio::task::JoinSet::new();
    for _ in 0..128 {
        let state = Arc::clone(&state);
        calls.spawn(async move {
            state
                .hub
                .call(
                    "node-report-progress",
                    NodeOperation::Maintenance {
                        command: ExecutionCommand {
                            action: "status".into(),
                            input: serde_json::json!({}),
                        },
                    },
                )
                .await
        });
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while tx.capacity() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (mut writer, mut written) = futures::channel::mpsc::unbounded();
    assert!(tokio::time::timeout(
        Duration::from_secs(2),
        finish(
            &state,
            "node-report-progress",
            generation,
            2,
            &report(true),
            &mut writer,
        )
    )
    .await
    .expect("handoff must not wait for its own full RPC queue")
    .unwrap());
    let Message::Text(text) = written.next().await.unwrap() else {
        panic!("expected handoff frame")
    };
    let ServerFrame::Call { operation, .. } = serde_json::from_str(&text).unwrap();
    let NodeOperation::Maintenance { command } = operation else {
        panic!("expected maintenance")
    };
    assert_eq!(command.action, "host_handoff_ready");
    assert_eq!(command.input["server"], "legacy");
    assert_eq!(tx.capacity(), 0, "handoff must preserve all queued RPCs");
    for _ in 0..128 {
        let SocketCommand::Frame(frame) = rx.recv().await.unwrap() else {
            panic!("expected RPC")
        };
        let ServerFrame::Call { request_id, .. } = *frame;
        state
            .hub
            .resolve(
                "node-report-progress",
                generation,
                &request_id,
                RpcReply::ok(serde_json::json!({})),
            )
            .await;
    }
    while let Some(result) = calls.join_next().await {
        assert_eq!(result.unwrap().status, 200);
    }
}

#[tokio::test]
async fn subsequent_report_does_not_repeat_handoff() {
    let root = tempfile::tempdir().unwrap();
    let _scope = opencoder_core::config::scoped_config_home(root.path().join("config"));
    let state = crate::new_state(root.path().join("work"), root.path().join("data"), None)
        .await
        .unwrap();
    let generation = "host-00000000000000000001-progress";
    let (_tx, _rx) = attach(&state, generation).await;
    let (mut writer, mut written) = futures::channel::mpsc::unbounded();
    assert!(finish(
        &state,
        "node-report-progress",
        generation,
        2,
        &report(false),
        &mut writer
    )
    .await
    .unwrap());
    drop(writer);
    assert!(written.next().await.is_none());
}

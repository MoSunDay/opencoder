//! Negotiated reports exercise the real socket, writer transaction and shared fence.
use super::report_storage::{Fixture, index, send, snapshot};
use futures::StreamExt;
use opencoder_core::fleet::*;
use std::time::Duration;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

pub(super) async fn ack(socket: &mut Socket, id: u64, sequence: u64) -> bool {
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(Ok(message)) = socket.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let ServerFrame::Call { operation, .. } = serde_json::from_str(&text).unwrap();
            if let NodeOperation::Maintenance { command } = operation {
                if command.action == INDEX_REPORT_ACK_ACTION {
                    assert_eq!(command.input["report_id"], id);
                    assert_eq!(command.input["sequence"], sequence);
                    return true;
                }
            }
        }
        false
    })
    .await
    .expect("commit acknowledgement timed out")
}
pub(super) async fn begin(socket: &mut Socket, id: u64, sequence: u64, base: Option<u64>) {
    send(
        socket,
        NodeFrame::Snapshot {
            snapshot: snapshot(sequence),
        },
    )
    .await;
    let mut report = IndexReportEnvelope::begin(id).with_ack(true);
    report.base_sequence = base;
    report.mode = if base.is_some() {
        IndexReportMode::Delta
    } else {
        IndexReportMode::Full
    };
    send(socket, NodeFrame::IndexReport { report }).await;
    assert!(ack(socket, id, 0).await);
}
pub(super) async fn batch(
    socket: &mut Socket,
    id: u64,
    records: Vec<ExecutionIndex>,
    removed: Vec<String>,
) {
    let mut report = IndexReportEnvelope::batch(id, records).with_ack(true);
    report.removed_ids = removed;
    send(socket, NodeFrame::IndexReport { report }).await;
    assert!(ack(socket, id, 1).await);
}
pub(super) async fn end(socket: &mut Socket, id: u64) -> bool {
    send(
        socket,
        NodeFrame::IndexReport {
            report: IndexReportEnvelope::end(id).with_ack(true),
        },
    )
    .await;
    ack(socket, id, 2).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shared_base_change_rebuilds_full_indexes_without_losing_pending_rpc_or_history() {
    let fixture = Fixture::start().await;
    let mut socket = fixture.open_mode(1, true).await;
    let first = index(ExecutionStatus::Running);
    let other = ExecutionIndex {
        id: "dag-other".into(),
        status: ExecutionStatus::Done,
        ..first.clone()
    };
    let unchanged = ExecutionIndex {
        id: "dag-unchanged".into(),
        ..first.clone()
    };
    begin(&mut socket, 1, 10, None).await;
    batch(
        &mut socket,
        1,
        vec![first.clone(), other.clone(), unchanged.clone()],
        vec![],
    )
    .await;
    assert!(end(&mut socket, 1).await);
    let pending = ExecutionIndex {
        id: "dag-pending-later".into(),
        status: ExecutionStatus::Pending,
        ..first.clone()
    };
    fixture.state.fleet.put_index(&pending).await.unwrap();
    begin(&mut socket, 2, 20, Some(10)).await;
    batch(
        &mut socket,
        2,
        vec![index(ExecutionStatus::Done)],
        vec![other.id.clone()],
    )
    .await;
    assert!(end(&mut socket, 2).await);
    assert_eq!(
        fixture
            .state
            .fleet
            .index(&pending.id)
            .await
            .unwrap()
            .unwrap(),
        pending
    );
    assert_eq!(
        fixture.state.fleet.index(&other.id).await.unwrap().unwrap(),
        other
    );
    let committed = fixture.state.fleet.index(&first.id).await.unwrap().unwrap();
    assert_eq!(committed.status, ExecutionStatus::Done);
    let generation = snapshot(1).generation;
    let mut other_snapshot = unchanged.clone();
    other_snapshot.status = ExecutionStatus::Done;
    fixture
        .state
        .fleet
        .apply_index_report_fenced(
            "node-storage",
            &[committed.clone(), other_snapshot],
            None,
            Some((&generation, 30)),
        )
        .await
        .unwrap();
    let state = fixture.state.clone();
    let pending_call = tokio::spawn(async move {
        state
            .hub
            .call(
                "node-storage",
                NodeOperation::Maintenance {
                    command: ExecutionCommand {
                        action: "held-test-call".into(),
                        input: serde_json::json!({}),
                    },
                },
            )
            .await
    });
    let request = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let Message::Text(text) = socket.next().await.unwrap().unwrap() else { continue };
            let ServerFrame::Call { request_id, operation } = serde_json::from_str(&text).unwrap();
            if matches!(operation, NodeOperation::Maintenance { command } if command.action == "held-test-call") {
                break request_id;
            }
        }
    }).await.unwrap();
    begin(&mut socket, 3, 31, Some(20)).await;
    batch(&mut socket, 3, vec![first.clone()], vec![]).await;
    assert!(end(&mut socket, 3).await);
    assert!(
        !pending_call.is_finished(),
        "resynchronizing indexes must not cancel a pending call"
    );
    send(
        &mut socket,
        NodeFrame::Reply {
            request_id: request,
            reply: RpcReply::ok(serde_json::json!({"kept":true})),
        },
    )
    .await;
    assert_eq!(pending_call.await.unwrap().status, 200);
    assert_eq!(
        fixture.state.fleet.index(&first.id).await.unwrap().unwrap(),
        first
    );
    assert_eq!(
        fixture
            .state
            .fleet
            .index(&unchanged.id)
            .await
            .unwrap()
            .unwrap(),
        unchanged,
        "an unchanged local field must be reconstructed, not left at another Server's value"
    );
    assert_eq!(
        fixture
            .state
            .fleet
            .index(&pending.id)
            .await
            .unwrap()
            .unwrap(),
        pending
    );
    assert!(fixture.state.hub.views().await[0].online);
    begin(&mut socket, 4, 40, Some(31)).await;
    batch(&mut socket, 4, vec![index(ExecutionStatus::Done)], vec![]).await;
    assert!(end(&mut socket, 4).await);
    socket.close(None).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn final_delta_ack_waits_for_storage_and_brain_does_not_consume_fragment_sequence() {
    let fixture = Fixture::start().await;
    let mut socket = fixture.open_mode(1, true).await;
    begin(&mut socket, 1, 10, None).await;
    batch(
        &mut socket,
        1,
        vec![index(ExecutionStatus::Running)],
        vec![],
    )
    .await;
    assert!(end(&mut socket, 1).await);
    begin(&mut socket, 2, 20, Some(10)).await;
    send(
        &mut socket,
        NodeFrame::Brain {
            execution: index(ExecutionStatus::Running).execution_ref(),
            action: "unknown-test-action".into(),
            input: serde_json::Value::Null,
        },
    )
    .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(30), socket.next())
            .await
            .is_err(),
        "standalone Brain must not generate a report fragment ACK"
    );
    batch(&mut socket, 2, vec![index(ExecutionStatus::Done)], vec![]).await;
    let writer = fixture.writer().await;
    send(
        &mut socket,
        NodeFrame::IndexReport {
            report: IndexReportEnvelope::end(2).with_ack(true),
        },
    )
    .await;
    fixture.blocked().await;
    assert!(
        tokio::time::timeout(Duration::from_millis(30), socket.next())
            .await
            .is_err()
    );
    assert_eq!(
        fixture
            .state
            .fleet
            .index("dag-storage")
            .await
            .unwrap()
            .unwrap()
            .status,
        ExecutionStatus::Running
    );
    writer.execute("COMMIT", ()).await.unwrap();
    assert!(ack(&mut socket, 2, 2).await);
    assert_eq!(
        fixture
            .state
            .fleet
            .index("dag-storage")
            .await
            .unwrap()
            .unwrap()
            .status,
        ExecutionStatus::Done
    );
    socket.close(None).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn superseded_reports_acknowledge_without_overwriting_newer_durable_indexes() {
    let fixture = Fixture::start().await;
    let mut socket = fixture.open_mode(1, true).await;
    let running = index(ExecutionStatus::Running);
    let done = index(ExecutionStatus::Done);
    begin(&mut socket, 1, 10, None).await;
    batch(&mut socket, 1, vec![running.clone()], vec![]).await;
    assert!(end(&mut socket, 1).await);
    let generation = snapshot(1).generation;
    fixture
        .state
        .fleet
        .apply_index_report_fenced(
            &done.node_id,
            std::slice::from_ref(&done),
            None,
            Some((&generation, 30)),
        )
        .await
        .unwrap();
    // A coexisting Server has already stored a more recent snapshot. ACK the
    // older local report so the connection remains usable, but do not regress it.
    begin(&mut socket, 2, 20, Some(10)).await;
    batch(&mut socket, 2, vec![running.clone()], vec![]).await;
    assert!(end(&mut socket, 2).await);
    assert_eq!(
        fixture.state.fleet.index(&done.id).await.unwrap().unwrap(),
        done
    );
    // The local cache still advances after the skipped report. The next report
    // can reconstruct its own full snapshot and commit once it is newer.
    begin(&mut socket, 3, 31, Some(20)).await;
    batch(&mut socket, 3, vec![], vec![]).await;
    assert!(end(&mut socket, 3).await);
    assert_eq!(
        fixture.state.fleet.index(&done.id).await.unwrap().unwrap(),
        running
    );
    fixture
        .state
        .fleet
        .apply_index_report_fenced(
            &done.node_id,
            std::slice::from_ref(&done),
            None,
            Some((&format!("{generation}-newer"), 1)),
        )
        .await
        .unwrap();
    begin(&mut socket, 4, 40, Some(31)).await;
    batch(&mut socket, 4, vec![running], vec![]).await;
    assert!(end(&mut socket, 4).await);
    assert_eq!(
        fixture.state.fleet.index(&done.id).await.unwrap().unwrap(),
        done
    );
    assert!(fixture.state.hub.views().await[0].online);
    socket.close(None).await.unwrap();
}

//! A locally superseded Host must finish its report flow while RPCs drain.
use super::incremental::{ack, batch, begin, end};
use super::report_storage::{index, send, Fixture};
use futures::StreamExt;
use opencoder_core::fleet::*;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn locally_superseded_host_report_is_acknowledged_without_replacing_new_owner() {
    let fixture = Fixture::start().await;
    let mut old = fixture.open_mode(1, true).await;
    begin(&mut old, 1, 10, None).await;
    batch(&mut old, 1, vec![index(ExecutionStatus::Running)], vec![]).await;
    assert!(end(&mut old, 1).await);
    let state = fixture.state.clone();
    let pending = tokio::spawn(async move {
        state
            .hub
            .call(
                "node-storage",
                NodeOperation::Maintenance {
                    command: ExecutionCommand {
                        action: "held-handoff-call".into(),
                        input: serde_json::json!({}),
                    },
                },
            )
            .await
    });
    let request_id = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let Message::Text(text) = old.next().await.unwrap().unwrap() else { continue };
            let ServerFrame::Call { request_id, operation } = serde_json::from_str(&text).unwrap();
            if matches!(operation, NodeOperation::Maintenance { command } if command.action == "held-handoff-call") {
                break request_id;
            }
        }
    }).await.unwrap();
    let generation = "host-00000000000000000002-storage";
    let mut new = fixture.open_generation(1, true, generation).await;
    send(
        &mut new,
        NodeFrame::IndexReport {
            report: IndexReportEnvelope::begin(1).with_ack(true),
        },
    )
    .await;
    assert!(ack(&mut new, 1, 0).await);
    let done = index(ExecutionStatus::Done);
    batch(&mut new, 1, vec![done.clone()], vec![]).await;
    assert!(end(&mut new, 1).await);
    begin(&mut old, 2, 20, Some(10)).await;
    batch(&mut old, 2, vec![index(ExecutionStatus::Running)], vec![]).await;
    assert!(end(&mut old, 2).await);
    assert_eq!(
        fixture.state.fleet.index(&done.id).await.unwrap().unwrap(),
        done
    );
    let view = fixture.state.hub.views().await.remove(0);
    assert!(view.online);
    assert_eq!(view.snapshot.unwrap().generation, generation);
    assert!(!pending.is_finished());
    send(
        &mut old,
        NodeFrame::Reply {
            request_id,
            reply: RpcReply::ok(serde_json::json!({"kept": true})),
        },
    )
    .await;
    assert_eq!(pending.await.unwrap().status, 200);
    old.close(None).await.unwrap();
    new.close(None).await.unwrap();
}

use super::report::{BeginDecision, ReportCollector};
use super::SocketCommand;
use crate::AppState;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use opencoder_core::fleet::*;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct NodeChannelQuery {
    node_id: String,
}

pub async fn upgrade(
    State(state): State<Arc<AppState>>,
    Query(query): Query<NodeChannelQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.max_message_size(MAX_FRAME_BYTES)
        .max_frame_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| async move {
            if let Err(error) = serve(state, socket, query.node_id).await {
                tracing::error!(%error, "node channel failed");
            }
        })
}

async fn serve(
    state: Arc<AppState>,
    socket: WebSocket,
    authenticated_node_id: String,
) -> anyhow::Result<()> {
    let (mut writer, mut reader) = socket.split();
    let first = tokio::time::timeout(std::time::Duration::from_secs(10), reader.next()).await?;
    let Some(Ok(Message::Text(text))) = first else {
        anyhow::bail!("node hello required");
    };
    let NodeFrame::Hello {
        registration,
        snapshot,
    } = serde_json::from_str(&text)?
    else {
        anyhow::bail!("node hello required");
    };
    if !authenticated_identity_matches(&authenticated_node_id, &registration.id) {
        anyhow::bail!("authenticated node_id does not match hello registration id");
    }
    let id = registration.id.clone();
    let generation = snapshot.generation.clone();
    let mut reports = ReportCollector::default();
    let mut initial_freeze_request = None;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<SocketCommand>(128);
    {
        // Admission transitions and attaches share this lock. A node joining a
        // frozen server is queued a Freeze before a concurrent Reopen can be
        // queued, while a node joining after Reopen observes Open.
        let _transition = state.admission.transition().await;
        state
            .hub
            .attach(registration.clone(), snapshot, tx.clone())
            .await?;
        if !state.admission.is_open().await {
            let request_id = ulid::Ulid::new().to_string();
            tx.try_send(SocketCommand::Frame(Box::new(ServerFrame::Call {
                request_id: request_id.clone(),
                operation: NodeOperation::Admission {
                    command: NodeAdmissionCommand::Freeze,
                },
            })))?;
            initial_freeze_request = Some(request_id);
        }
    }
    let outcome = async {
        state.fleet.register(&registration).await?;
        loop {
            tokio::select! {
                Some(frame) = rx.recv() => {
                    let SocketCommand::Frame(frame) = frame else {
                        writer.send(Message::Close(None)).await?;
                        break;
                    };
                    let text = serde_json::to_string(frame.as_ref())?;
                    if text.len() > MAX_FRAME_BYTES { anyhow::bail!("server frame exceeds limit"); }
                    tokio::time::timeout(std::time::Duration::from_secs(5), writer.send(Message::Text(text))).await??;
                }
                incoming = tokio::time::timeout(std::time::Duration::from_millis(STALE_MS as u64), reader.next()) => {
                    let Some(incoming) = incoming? else { break; };
                    let frame = match incoming? {
                        Message::Text(text) => serde_json::from_str::<NodeFrame>(&text)?,
                        Message::Close(_) => break,
                        Message::Ping(data) => { writer.send(Message::Pong(data)).await?; continue; }
                        _ => anyhow::bail!("invalid node frame"),
                    };
                    match frame {
                        NodeFrame::Hello { .. } => anyhow::bail!("duplicate hello"),
                        NodeFrame::Snapshot { snapshot } => state.hub.snapshot(&id, &generation, snapshot).await,
                        NodeFrame::IndexReport { report } => {
                            if !state.hub.touch(&id, &generation).await { continue; }
                            let report_id = report.report_id;
                            match report.part {
                                IndexReportPart::Begin => match reports.begin(report_id)? {
                                    BeginDecision::CapturePending => {
                                        let pending = state.fleet.pending_ids(&id).await?;
                                        reports.set_pending_at_begin(report_id, pending)?;
                                    }
                                    BeginDecision::Started | BeginDecision::Ignore => {}
                                },
                                IndexReportPart::Batch { records } => reports.batch(report_id, records)?,
                                IndexReportPart::End => {
                                    if let Some(complete) = reports.end(report_id)? {
                                        let recovered = state.fleet.apply_index_report(
                                            &id,
                                            &complete.records,
                                            complete.pending_at_begin.as_deref(),
                                        ).await?;
                                        state.hub.acknowledge_report(&id, &complete.records).await;
                                        // Initial sync settles claims whose old connection can no
                                        // longer reply. Live calls settle only after the Node's
                                        // ordered post-operation snapshot precedes its reply.
                                        if complete.initial {
                                            state.hub.clear_reservations(&complete.records).await;
                                        }
                                        state.hub.clear_reservations(&recovered).await;
                                        if !state.hub.mark_index_synced(&id, &generation).await {
                                            anyhow::bail!("index report belongs to a stale connection");
                                        }
                                        tracing::debug!(node_id = %id, report_id = complete.report_id, records = complete.records.len(), "index report applied");
                                    }
                                }
                            }
                        }
                        NodeFrame::Reply { request_id, reply } => {
                            if initial_freeze_request.as_deref() == Some(request_id.as_str()) {
                                initial_freeze_request = None;
                                if !(200..300).contains(&reply.status) {
                                    tracing::warn!(node_id = %id, status = reply.status, body = %reply.body, "freeze joining node while server admission is frozen");
                                }
                                continue;
                            }
                            if let Some(create) = state.hub.resolve(&id, &generation, &request_id, reply).await {
                                settle_create_reply(&state, create).await?;
                            }
                        }
                    }
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    }.await;
    state.hub.detach(&id, &generation).await;
    outcome
}

async fn settle_create_reply(
    state: &AppState,
    create: super::hub::CreateReply,
) -> anyhow::Result<()> {
    let index = create.execution;
    settle_create_index(
        state.fleet.as_ref(),
        &index,
        &create.reply,
        create.reject_pending,
    )
    .await?;
    tracing::debug!(execution_id = %index.id, status = create.reply.status, late = create.late, "create reply settled");
    Ok(())
}

async fn settle_create_index(
    fleet: &opencoder_store::fleet::FleetStore,
    index: &ExecutionIndex,
    reply: &RpcReply,
    reject_pending: bool,
) -> anyhow::Result<()> {
    if (200..300).contains(&reply.status) {
        let accepted: ExecutionIndex = serde_json::from_value(reply.body.clone())?;
        if accepted.id != index.id
            || accepted.node_id != index.node_id
            || accepted.created_at != index.created_at
            || accepted.kind != index.kind
        {
            anyhow::bail!("invalid late node acceptance");
        }
    } else if reject_pending
        && rejection_proves_not_accepted(reply.status)
        && fleet
            .index(&index.id)
            .await?
            .is_some_and(|current| current.status == ExecutionStatus::Pending)
    {
        fleet
            .put_index(&ExecutionIndex {
                status: ExecutionStatus::Error,
                ..index.clone()
            })
            .await?;
    }
    Ok(())
}

fn rejection_proves_not_accepted(status: u16) -> bool {
    // These Create responses are emitted before the Node writes its durable
    // journal record. A 409 may describe an existing acceptance, 428 triggers
    // the definition retry, and 5xx can follow a journal write that succeeded.
    matches!(status, 400 | 429 | 503)
}

fn authenticated_identity_matches(authenticated_node_id: &str, hello_node_id: &str) -> bool {
    authenticated_node_id == hello_node_id
}

#[cfg(test)]
mod tests {
    use super::{authenticated_identity_matches, settle_create_index};
    use opencoder_core::fleet::{ExecutionIndex, ExecutionKind, ExecutionStatus, RpcReply};
    use opencoder_store::fleet::FleetStore;

    #[test]
    fn authenticated_query_identity_must_equal_hello_identity() {
        assert!(authenticated_identity_matches("node-a", "node-a"));
        assert!(!authenticated_identity_matches("node-a", "node-b"));
    }

    #[tokio::test]
    async fn definitive_failed_create_reply_converges_pending_index() {
        let fleet = FleetStore::open_memory().await.unwrap();
        let index = ExecutionIndex {
            id: "agent-late-failure".into(),
            created_at: 9,
            kind: ExecutionKind::Agent,
            node_id: "node-a".into(),
            status: ExecutionStatus::Pending,
        };
        fleet.put_index(&index).await.unwrap();
        settle_create_index(&fleet, &index, &RpcReply::error(400, "failed"), true)
            .await
            .unwrap();
        assert_eq!(
            fleet.index(&index.id).await.unwrap().unwrap().status,
            ExecutionStatus::Error
        );
    }

    #[tokio::test]
    async fn ambiguous_failure_keeps_pending_for_full_report_recovery() {
        let fleet = FleetStore::open_memory().await.unwrap();
        let index = ExecutionIndex {
            id: "agent-ambiguous-failure".into(),
            created_at: 10,
            kind: ExecutionKind::Agent,
            node_id: "node-a".into(),
            status: ExecutionStatus::Pending,
        };
        fleet.put_index(&index).await.unwrap();
        settle_create_index(&fleet, &index, &RpcReply::error(500, "failed"), true)
            .await
            .unwrap();
        assert_eq!(
            fleet.index(&index.id).await.unwrap().unwrap().status,
            ExecutionStatus::Pending
        );
    }

    #[tokio::test]
    async fn replies_do_not_regress_reports_or_reject_an_inflight_peer() {
        let fleet = FleetStore::open_memory().await.unwrap();
        let pending = ExecutionIndex {
            id: "agent-race".into(),
            created_at: 11,
            kind: ExecutionKind::Agent,
            node_id: "node-a".into(),
            status: ExecutionStatus::Pending,
        };
        fleet.put_index(&pending).await.unwrap();
        settle_create_index(
            &fleet,
            &pending,
            &RpcReply::error(500, "another create is in flight"),
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            fleet.index(&pending.id).await.unwrap().unwrap().status,
            ExecutionStatus::Pending
        );

        fleet
            .put_index(&ExecutionIndex {
                status: ExecutionStatus::Running,
                ..pending.clone()
            })
            .await
            .unwrap();
        settle_create_index(
            &fleet,
            &pending,
            &RpcReply::ok(serde_json::to_value(&pending).unwrap()),
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            fleet.index(&pending.id).await.unwrap().unwrap().status,
            ExecutionStatus::Running
        );

        fleet
            .put_index(&ExecutionIndex {
                status: ExecutionStatus::Done,
                ..pending.clone()
            })
            .await
            .unwrap();
        settle_create_index(
            &fleet,
            &pending,
            &RpcReply::ok(serde_json::to_value(&pending).unwrap()),
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            fleet.index(&pending.id).await.unwrap().unwrap().status,
            ExecutionStatus::Done
        );
    }
}

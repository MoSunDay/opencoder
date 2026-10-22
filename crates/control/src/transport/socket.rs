use super::SocketCommand;
use super::report::{BeginDecision, ReportCollector};
use crate::AppState;
use axum::{
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
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
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let incremental = headers
        .get(INDEX_REPORT_DELTA_HEADER)
        .is_some_and(|v| v == INDEX_REPORT_DELTA_VERSION)
        && headers
            .get(INDEX_REPORT_ACK_HEADER)
            .is_some_and(|v| v == "1");
    let mut response = ws
        .max_message_size(MAX_FRAME_BYTES)
        .max_frame_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| async move {
            let node_id = query.node_id;
            if let Err(error) = serve(state, socket, node_id.clone(), incremental).await {
                tracing::error!(%node_id, %error, "node channel failed");
            }
        });
    response.headers_mut().insert(
        INDEX_REPORT_ACK_HEADER,
        axum::http::HeaderValue::from_static("1"),
    );
    if incremental {
        response.headers_mut().insert(
            INDEX_REPORT_DELTA_HEADER,
            axum::http::HeaderValue::from_static(INDEX_REPORT_DELTA_VERSION),
        );
    }
    response
}

async fn serve(
    state: Arc<AppState>,
    socket: WebSocket,
    authenticated_node_id: String,
    incremental: bool,
) -> anyhow::Result<()> {
    let (mut writer, mut reader) = socket.split();
    let first = tokio::select! {
        _ = state.lifecycle.channels_retired() => {
            writer.send(Message::Close(None)).await?;
            return Ok(());
        }
        first = tokio::time::timeout(std::time::Duration::from_secs(10), reader.next()) => first?,
    };
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
    anyhow::ensure!(
        !incremental || generation.starts_with("host-"),
        "incremental reports require a managed Host generation"
    );
    let mut snapshot_sequence = snapshot.sequence;
    let mut report_sequence = snapshot_sequence;
    let mut report_flow: Option<(u64, u64)> = None;
    let mut reports = ReportCollector::default();
    let brain_deliveries = super::brain_delivery::InFlight::default();
    let mut initial_admission_request;
    let (tx, rx) = tokio::sync::mpsc::channel::<SocketCommand>(128);
    {
        // Admission transitions and attaches share this lock. A node joining a
        // frozen server is queued a Freeze before a concurrent Reopen can be
        // queued, while a node joining after Reopen is sent Reopen.
        let _transition = state.admission.transition().await;
        // Local shutdown persists Frozen. Reconcile both modes on reconnect
        // so a restarted node can rejoin an open cluster, even when another
        // resource error masks its local admission state in the snapshot.
        let command = if generation.starts_with("host-") {
            NodeAdmissionCommand::Status
        } else if state.admission.is_open().await {
            NodeAdmissionCommand::Reopen
        } else {
            NodeAdmissionCommand::Freeze
        };
        state
            .hub
            .attach(registration.clone(), snapshot, tx.clone())
            .await?;
        let request_id = ulid::Ulid::new().to_string();
        tx.try_send(SocketCommand::Frame(Box::new(ServerFrame::Call {
            request_id: request_id.clone(),
            operation: NodeOperation::Admission { command },
        })))?;
        initial_admission_request = Some((request_id, command));
    }
    let (acknowledgements, ack_rx) = tokio::sync::mpsc::channel(1);
    let (input, mut incoming) = tokio::sync::mpsc::channel(128);
    let mut transport = tokio::task::JoinSet::new();
    transport.spawn(
        super::socket_io::Transport {
            writer,
            reader,
            commands: rx,
            acknowledgements: ack_rx,
            incoming: input,
        }
        .run(state.clone(), id.clone(), generation.clone()),
    );
    let process = async {
        let fleet = state.fleet.clone();
        let node = registration.clone();
        super::socket_io::storage(async move { fleet.register(&node).await }).await?;
        let mut applying = tokio::task::JoinSet::<anyhow::Result<Option<u64>>>::new();
        loop {
            // Transport owns termination and its precise I/O error. Once the
            // input queue closes, let the outer select collect that result.
            let frame = tokio::select! {
                finished = applying.join_next(), if !applying.is_empty() => {
                    if let Some(sequence) = finished.expect("one report application")?? {
                        reports.committed(sequence);
                    }
                    continue;
                }
                incoming = incoming.recv() => match incoming {
                    Some(frame) => frame,
                    None => break std::future::pending::<anyhow::Result<()>>().await,
                },
            };
            match frame {
                NodeFrame::Brain {
                    execution,
                    action,
                    input,
                } => {
                    if !state.hub.touch(&id, &generation).await {
                        continue;
                    }
                    if let Some(delivery) = brain_deliveries.start(&execution, &action, &input) {
                        let state = state.clone();
                        let node = id.clone();
                        tokio::spawn(async move {
                            let _delivery = delivery;
                            if let Err(error) = crate::api::brain_runs::effects::deliver(
                                state, node, execution, action, input,
                            )
                            .await
                            {
                                tracing::warn!(%error,"brain delivery remains pending for replay");
                            }
                        });
                    }
                    if let Some((report_id, sequence)) =
                        report_flow.as_mut().filter(|_| !incremental)
                    {
                        *sequence = sequence
                            .checked_add(1)
                            .ok_or_else(|| anyhow::anyhow!("report acknowledgement overflow"))?;
                        super::socket_report::acknowledge(&acknowledgements, *report_id, *sequence)
                            .await?;
                    }
                }
                NodeFrame::Hello { .. } => anyhow::bail!("duplicate hello"),
                NodeFrame::Snapshot { snapshot } => {
                    snapshot_sequence = snapshot.sequence;
                    state.hub.snapshot(&id, &generation, snapshot).await;
                }
                NodeFrame::IndexReport { report } => {
                    // A replaced Host still drains its original RPCs. Complete
                    // report flow control on that socket; storage and Hub fences
                    // reject its obsolete data without causing an ACK timeout.
                    state.hub.touch(&id, &generation).await;
                    if !applying.is_empty() {
                        if let Some(sequence) = applying
                            .join_next()
                            .await
                            .expect("one report application")??
                        {
                            reports.committed(sequence);
                        }
                    }
                    let report_id = report.report_id;
                    let acknowledged = report.acknowledge;
                    anyhow::ensure!(
                        !incremental || acknowledged,
                        "negotiated reports require commit acknowledgements"
                    );
                    anyhow::ensure!(
                        matches!(report.part, IndexReportPart::Begin)
                            || (report.mode == IndexReportMode::Full
                                && report.base_sequence.is_none()),
                        "report metadata is only legal at begin"
                    );
                    anyhow::ensure!(
                        matches!(report.part, IndexReportPart::Batch { .. })
                            || report.removed_ids.is_empty(),
                        "removals are only legal in batches"
                    );
                    match report.part {
                        IndexReportPart::Begin => match reports.begin_sync(
                            report_id,
                            incremental,
                            report.mode,
                            report.base_sequence,
                        )? {
                            BeginDecision::CapturePending => {
                                report_sequence = snapshot_sequence;
                                let fleet = state.fleet.clone();
                                let node = id.clone();
                                let started = std::time::Instant::now();
                                let pending = super::socket_io::storage(async move {
                                    fleet.pending_ids(&node).await
                                })
                                .await?;
                                if started.elapsed().as_millis() >= 250 {
                                    tracing::info!(node_id = %id, report_id, pending = pending.len(),
                                            elapsed_ms = started.elapsed().as_millis(),
                                            "control initial report capture timing");
                                }
                                reports.set_pending_at_begin(report_id, pending)?;
                                report_flow = acknowledged.then_some((report_id, 0));
                                if acknowledged {
                                    super::socket_report::acknowledge(
                                        &acknowledgements,
                                        report_id,
                                        0,
                                    )
                                    .await?;
                                }
                            }
                            BeginDecision::Started => {
                                report_sequence = snapshot_sequence;
                                report_flow = acknowledged.then_some((report_id, 0));
                                if acknowledged {
                                    super::socket_report::acknowledge(
                                        &acknowledgements,
                                        report_id,
                                        0,
                                    )
                                    .await?;
                                }
                            }
                            BeginDecision::Ignore => {}
                        },
                        IndexReportPart::Batch { records } => {
                            anyhow::ensure!(
                                records.len() + report.removed_ids.len() <= INDEX_REPORT_BATCH_SIZE,
                                "report batch too large"
                            );
                            reports.batch(report_id, records)?;
                            reports.removals(report_id, report.removed_ids)?;
                            if let Some((active, sequence)) = report_flow.as_mut() {
                                anyhow::ensure!(
                                    *active == report_id && acknowledged,
                                    "report acknowledgement ownership mismatch"
                                );
                                *sequence = sequence.checked_add(1).ok_or_else(|| {
                                    anyhow::anyhow!("report acknowledgement overflow")
                                })?;
                                super::socket_report::acknowledge(
                                    &acknowledgements,
                                    report_id,
                                    *sequence,
                                )
                                .await?;
                            }
                        }
                        IndexReportPart::End => {
                            if let Some(complete) = reports.end(report_id)? {
                                if let Some((active, sequence)) = report_flow.take() {
                                    anyhow::ensure!(
                                        active == report_id && acknowledged,
                                        "report acknowledgement ownership mismatch"
                                    );
                                    let sequence = sequence.checked_add(1).ok_or_else(|| {
                                        anyhow::anyhow!("report acknowledgement overflow")
                                    })?;
                                    let state = state.clone();
                                    let id = id.clone();
                                    let generation = generation.clone();
                                    let sender = acknowledgements.clone();
                                    // One connection-owned application preserves ordering while
                                    // the same socket still resolves replies and reads load updates.
                                    applying.spawn(async move {
                                        super::socket_report::finish(
                                            &state,
                                            &id,
                                            &generation,
                                            report_sequence,
                                            &complete,
                                            &sender,
                                        )
                                        .await?;
                                        // A superseded generation still owns its outstanding
                                        // RPC replies. Acknowledge the discarded report so its
                                        // flow-control timeout cannot sever that reply path.
                                        super::socket_report::acknowledge(
                                            &sender, report_id, sequence,
                                        )
                                        .await?;
                                        Ok(Some(report_sequence))
                                    });
                                } else {
                                    let applied = super::socket_report::finish(
                                        &state,
                                        &id,
                                        &generation,
                                        report_sequence,
                                        &complete,
                                        &acknowledgements,
                                    )
                                    .await?;
                                    if applied {
                                        reports.committed(report_sequence);
                                    }
                                }
                            }
                        }
                    }
                }
                NodeFrame::Reply { request_id, reply } => {
                    if initial_admission_request
                        .as_ref()
                        .is_some_and(|(id, _)| id == &request_id)
                    {
                        let (_, command) = initial_admission_request.take().unwrap();
                        let expected = if command == NodeAdmissionCommand::Freeze {
                            "frozen"
                        } else {
                            "open"
                        };
                        if !(200..300).contains(&reply.status)
                            || (command != NodeAdmissionCommand::Status
                                && reply.body["mode"] != expected)
                        {
                            tracing::warn!(node_id = %id, ?command, status = reply.status, body = %reply.body, "joining node rejected admission synchronization");
                        }
                        continue;
                    }
                    if let Some(create) = state
                        .hub
                        .resolve(&id, &generation, &request_id, reply)
                        .await
                    {
                        let state = state.clone();
                        super::socket_io::storage(async move {
                            settle_create_reply(&state, create).await
                        })
                        .await?;
                    }
                }
            }
        }
    };
    let outcome = tokio::select! {
        result = process => result,
        result = transport.join_next() => match result.expect("one transport task") {
            Ok(result) => result,
            Err(error) => Err(error.into()),
        },
    };
    transport.abort_all();
    while transport.join_next().await.is_some() {}
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
        fleet.accept_execution(&accepted).await?;
    } else if !matches!(index.kind, ExecutionKind::Agent | ExecutionKind::Operator)
        && reject_pending
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
            id: "dag-late-failure".into(),
            created_at: 9,
            kind: ExecutionKind::Dag,
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

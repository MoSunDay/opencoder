use super::NodeService;
mod report;
use anyhow::{bail, Context, Result};
use futures::{FutureExt, SinkExt, StreamExt};
use opencoder_core::fleet::*;
use report::{begin_report, ReportFrames};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, protocol::frame::coding::CloseCode, Message},
};

pub async fn run(remote: &str, token: &str, service: Arc<dyn NodeService>) -> Result<()> {
    if token.trim().is_empty() {
        bail!("agent token required");
    }
    let mut url = reqwest::Url::parse(remote)?;
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        _ => bail!("remote must use http or https"),
    };
    url.set_scheme(scheme)
        .map_err(|_| anyhow::anyhow!("invalid remote scheme"))?;
    url.set_path("/api/nodes/channel");
    url.query_pairs_mut()
        .clear()
        .append_pair("node_id", &service.registration().id);
    let capacity = Arc::new(tokio::sync::Semaphore::new(128));
    loop {
        if !service.reconnect_allowed(remote).await? {
            return Ok(());
        }
        let outcome = connection(url.as_str(), token, service.clone(), capacity.clone()).await;
        if let Err(error) = outcome {
            tracing::error!(%error, "node channel disconnected; local execution continues");
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

async fn connection(
    url: &str,
    token: &str,
    service: Arc<dyn NodeService>,
    capacity: Arc<tokio::sync::Semaphore>,
) -> Result<()> {
    connection_with_period(
        url,
        token,
        service,
        capacity,
        std::time::Duration::from_millis(HEARTBEAT_MS),
    )
    .await
}

async fn connection_with_period(
    url: &str,
    token: &str,
    service: Arc<dyn NodeService>,
    capacity: Arc<tokio::sync::Semaphore>,
    period: std::time::Duration,
) -> Result<()> {
    let mut request = url.into_client_request()?;
    request.headers_mut().insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {token}").parse()?,
    );
    let (socket, response) =
        tokio::time::timeout(std::time::Duration::from_secs(15), connect_async(request)).await??;
    let flow_controlled = response
        .headers()
        .get(INDEX_REPORT_ACK_HEADER)
        .is_some_and(|value| value == "1");
    let (mut writer, mut reader) = socket.split();
    send(
        &mut writer,
        &NodeFrame::Hello {
            registration: service.registration(),
            snapshot: service.snapshot(),
        },
    )
    .await?;
    let (tx, mut rx) = mpsc::channel::<NodeFrame>(128);
    let (report_trigger, mut report_requests) = mpsc::channel::<()>(1);
    let (admission_tx, admission_rx) = mpsc::channel(128);
    let admission_capacity = Arc::new(tokio::sync::Semaphore::new(128));
    let mut admissions = tokio::task::JoinSet::new();
    admissions.spawn(execute_admissions(
        service.clone(),
        admission_rx,
        tx.clone(),
        report_trigger.clone(),
    ));
    // Admission is serialized on this connection. Collection must keep running
    // while an admission call waits for a lock held or reserved by its report.
    // JoinSet also cancels collection when this connection is closed.
    let mut reports = tokio::task::JoinSet::new();
    let mut report_inflight = false;
    let mut report_pending = false;
    let mut publishing: Option<ReportFrames> = None;
    report_trigger.try_send(())?;
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    let mut changes = service.changes();
    let mut next_report_id = 1u64;
    loop {
        tokio::select! {
            _ = tick.tick() => {
                if service.retiring() && capacity.available_permits() == 128
                    && admission_capacity.available_permits() == 128 && rx.is_empty()
                    && publishing.is_none() {
                    writer.send(Message::Close(None)).await?;
                    return Ok(());
                }
                // Inventory collection and admission may wait on slow disk/NFS.
                // Transport liveness must not wait for either, or fabricate a
                // fresh load/index report while they are still in progress.
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    writer.send(Message::Ping(Vec::new())),
                ).await??;
                request_report(&report_trigger);
            }
            Some(finished) = admissions.join_next() => {
                finished.context("node admission worker failed")?;
                bail!("node admission worker stopped");
            }
            changed = changes.changed() => {
                changed.context("node revision channel closed")?;
                request_report(&report_trigger);
            }
            Some(()) = report_requests.recv() => {
                if report_inflight {
                    report_pending = true;
                } else {
                    report_inflight = true;
                    reports.spawn(prepare_owned_report(service.clone()));
                }
            }
            Some(prepared) = reports.join_next(), if report_inflight => {
                match prepared.context("node inventory worker failed")? {
                    Ok(report) => {
                        publishing = Some(begin_report(&mut writer, report, next_report_id, flow_controlled).await?);
                    }
                    Err(error) => {
                        report_inflight = false;
                        tracing::warn!(%error, "node inventory unavailable; control channel retained");
                        // Preserve known load and indexes, but never advertise
                        // readiness or an empty complete report after a failed
                        // collection. The next heartbeat/change retries it.
                        let mut snapshot = service.snapshot();
                        snapshot.ready = false;
                        snapshot.resource_error = Some(error.to_string());
                        send(&mut writer, &NodeFrame::Snapshot { snapshot }).await?;
                    }
                }
                if !report_inflight && report_pending {
                    report_pending = false;
                    request_report(&report_trigger);
                }
            }
            _ = std::future::ready(()), if publishing.as_ref().is_some_and(ReportFrames::ready) => {
                let report = publishing.as_mut().expect("active report");
                if let Some(frame) = report.next() {
                    send(&mut writer, &frame).await?;
                    report.sent()?;
                    // Ready socket writes are not guaranteed to yield. Let RPC
                    // handlers enqueue their replies before the next batch.
                    tokio::task::yield_now().await;
                } else {
                    report.log_sent();
                    publishing = None;
                    report_inflight = false;
                    next_report_id = next_report_id.checked_add(1).context("index report id overflow")?;
                    if report_pending {
                        report_pending = false;
                        request_report(&report_trigger);
                    }
                }
            }
            Some(frame) = rx.recv() => send(&mut writer, &frame).await?,
            frame = reader.next() => {
                let Some(frame) = frame else { bail!("server closed channel"); };
                match frame? {
                    Message::Text(text) if text.len() <= MAX_FRAME_BYTES => match serde_json::from_str::<ServerFrame>(&text)? {
                        ServerFrame::Call { request_id, operation } => {
                            if let NodeOperation::Maintenance { command } = &operation {
                                if command.action == INDEX_REPORT_ACK_ACTION && flow_controlled {
                                    let report_id = command.input["report_id"].as_u64().context("report acknowledgement id missing")?;
                                    let sequence = command.input["sequence"].as_u64().context("report acknowledgement sequence missing")?;
                                    anyhow::ensure!(request_id == format!("report-ack-{report_id}-{sequence}"), "invalid transport acknowledgement identity");
                                    if let Some(report) = publishing.as_mut() { report.acknowledge(report_id, sequence)?; }
                                    continue;
                                }
                            }
                            if matches!(&operation, NodeOperation::Admission { .. }) {
                                let permit = admission_capacity.clone().try_acquire_owned()
                                    .context("node admission capacity exhausted")?;
                                admission_tx.try_send((request_id, operation, permit))
                                    .context("node admission queue unavailable")?;
                                continue;
                            }
                            let tx = tx.clone(); let service = service.clone();
                            let report_trigger = report_trigger.clone();
                            let permit = match capacity.clone().try_acquire_owned() {
                                Ok(permit) => permit,
                                Err(_) => {
                                tx.try_send(NodeFrame::Reply { request_id, reply: RpcReply::error(503, "node control channel busy") })?;
                                continue;
                                }
                            };
                            tokio::spawn(async move {
                                let _permit = permit;
                                execute_call(service, operation, request_id, tx, report_trigger)
                                    .await;
                            });
                        }
                    },
                    Message::Ping(data) => writer.send(Message::Pong(data)).await?,
                    Message::Pong(_) => {},
                    Message::Close(frame) => return close_outcome(frame.map(|frame| frame.code)),
                    _ => bail!("invalid server frame"),
                }
            }
        }
    }
}

fn close_outcome(code: Option<CloseCode>) -> Result<()> {
    match code {
        None | Some(CloseCode::Normal | CloseCode::Away) => Ok(()),
        Some(code) => bail!("server closed node channel with code {}", u16::from(code)),
    }
}

async fn execute_admissions(
    service: Arc<dyn NodeService>,
    mut requests: mpsc::Receiver<(String, NodeOperation, tokio::sync::OwnedSemaphorePermit)>,
    tx: mpsc::Sender<NodeFrame>,
    report_trigger: mpsc::Sender<()>,
) {
    // One connection-owned worker preserves FIFO admission transitions.
    // Its JoinSet aborts pending calls on disconnect, before a new connection
    // can apply its initial admission state.
    while let Some((request_id, operation, _permit)) = requests.recv().await {
        execute_call(
            service.clone(),
            operation,
            request_id,
            tx.clone(),
            report_trigger.clone(),
        )
        .await;
    }
}

async fn execute_call(
    service: Arc<dyn NodeService>,
    operation: NodeOperation,
    request_id: String,
    tx: mpsc::Sender<NodeFrame>,
    report_trigger: mpsc::Sender<()>,
) {
    let refresh = operation.refreshes_inventory();
    let reply = invoke(service.as_ref(), operation).await;
    if refresh {
        // Publish load before mutation replies release reservations. Reads
        // must not amplify polling into repeated whole-inventory reports.
        let _ = tx
            .send(NodeFrame::Snapshot {
                snapshot: service.snapshot(),
            })
            .await;
        request_report(&report_trigger);
    }
    let _ = tx.send(NodeFrame::Reply { request_id, reply }).await;
}

async fn invoke(service: &dyn NodeService, operation: NodeOperation) -> RpcReply {
    std::panic::AssertUnwindSafe(service.handle(operation))
        .catch_unwind()
        .await
        .unwrap_or_else(|_| RpcReply::error(500, "node operation panicked"))
}

fn request_report(trigger: &mpsc::Sender<()>) {
    let _ = trigger.try_send(());
}

type PreparedReport = super::NodeReport;

async fn prepare_report(service: &dyn NodeService) -> Result<PreparedReport> {
    service.report().await
}

async fn prepare_owned_report(service: Arc<dyn NodeService>) -> Result<PreparedReport> {
    let started = std::time::Instant::now();
    let result = prepare_report(service.as_ref()).await;
    if started.elapsed().as_millis() >= 250 {
        tracing::info!(node_id = %service.registration().id, elapsed_ms = started.elapsed().as_millis(),
            succeeded = result.is_ok(), "node inventory collection timing");
    }
    result
}

async fn send<S>(writer: &mut S, frame: &NodeFrame) -> Result<()>
where
    S: futures::Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    let text = serde_json::to_string(frame)?;
    if text.len() > MAX_FRAME_BYTES {
        bail!("node frame exceeds size limit");
    }
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        writer.send(Message::Text(text)),
    )
    .await??;
    Ok(())
}

#[cfg(test)]
mod tests;

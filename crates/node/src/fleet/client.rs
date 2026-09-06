use super::NodeService;
use anyhow::{bail, Context, Result};
use futures::{FutureExt, SinkExt, StreamExt};
use opencoder_core::fleet::*;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
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
    let mut request = url.into_client_request()?;
    request.headers_mut().insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {token}").parse()?,
    );
    let (socket, _) =
        tokio::time::timeout(std::time::Duration::from_secs(15), connect_async(request)).await??;
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
    let mut report = futures::future::pending::<Result<PreparedReport>>().boxed();
    let mut report_inflight = false;
    let mut report_pending = false;
    report_trigger.try_send(())?;
    let period = std::time::Duration::from_millis(HEARTBEAT_MS);
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    let mut changes = service.changes();
    let mut next_report_id = 1u64;
    loop {
        tokio::select! {
            _ = tick.tick() => {
                request_report(&report_trigger);
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
                    report = prepare_owned_report(service.clone()).boxed();
                }
            }
            prepared = &mut report, if report_inflight => {
                report_inflight = false;
                report = futures::future::pending::<Result<PreparedReport>>().boxed();
                publish_report(&mut writer, prepared?, next_report_id).await?;
                next_report_id = next_report_id.checked_add(1).context("index report id overflow")?;
                if report_pending {
                    report_pending = false;
                    request_report(&report_trigger);
                }
            }
            Some(frame) = rx.recv() => send(&mut writer, &frame).await?,
            frame = reader.next() => {
                let Some(frame) = frame else { bail!("server closed channel"); };
                match frame? {
                    Message::Text(text) if text.len() <= MAX_FRAME_BYTES => match serde_json::from_str::<ServerFrame>(&text)? {
                        ServerFrame::Call { request_id, operation } => {
                            if matches!(&operation, NodeOperation::Admission { .. }) {
                                execute_admission(
                                    &mut writer,
                                    service.as_ref(),
                                    operation,
                                    request_id,
                                    &report_trigger,
                                )
                                .await?;
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
                    Message::Close(_) => bail!("server closed channel"),
                    _ => bail!("invalid server frame"),
                }
            }
        }
    }
}

async fn execute_admission<S>(
    writer: &mut S,
    service: &dyn NodeService,
    operation: NodeOperation,
    request_id: String,
    report_trigger: &mpsc::Sender<()>,
) -> Result<()>
where
    S: futures::Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    let reply = invoke(service, operation).await;
    send(
        writer,
        &NodeFrame::Snapshot {
            snapshot: service.snapshot(),
        },
    )
    .await?;
    request_report(report_trigger);
    send(writer, &NodeFrame::Reply { request_id, reply }).await
}

async fn execute_call(
    service: Arc<dyn NodeService>,
    operation: NodeOperation,
    request_id: String,
    tx: mpsc::Sender<NodeFrame>,
    report_trigger: mpsc::Sender<()>,
) {
    let reply = invoke(service.as_ref(), operation).await;
    // Publish load before replies release reservations.
    let _ = tx
        .send(NodeFrame::Snapshot {
            snapshot: service.snapshot(),
        })
        .await;
    request_report(&report_trigger);
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

struct PreparedReport {
    snapshot: NodeSnapshot,
    records: Vec<ExecutionIndex>,
}

async fn prepare_report(service: &dyn NodeService) -> Result<PreparedReport> {
    let records = service.indexes().await?;
    Ok(PreparedReport {
        snapshot: service.snapshot(),
        records,
    })
}

async fn prepare_owned_report(service: Arc<dyn NodeService>) -> Result<PreparedReport> {
    prepare_report(service.as_ref()).await
}

async fn publish_report<S>(writer: &mut S, report: PreparedReport, report_id: u64) -> Result<()>
where
    S: futures::Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    // The control plane releases placement reservations after applying the
    // report. Publish the matching load snapshot first so accepted work is
    // never absent from both the snapshot and the reservation count.
    send(
        writer,
        &NodeFrame::Snapshot {
            snapshot: report.snapshot,
        },
    )
    .await?;
    send(
        writer,
        &NodeFrame::IndexReport {
            report: IndexReportEnvelope::begin(report_id),
        },
    )
    .await?;
    for batch in report.records.chunks(INDEX_REPORT_BATCH_SIZE) {
        send(
            writer,
            &NodeFrame::IndexReport {
                report: IndexReportEnvelope::batch(report_id, batch.to_vec()),
            },
        )
        .await?;
    }
    send(
        writer,
        &NodeFrame::IndexReport {
            report: IndexReportEnvelope::end(report_id),
        },
    )
    .await
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

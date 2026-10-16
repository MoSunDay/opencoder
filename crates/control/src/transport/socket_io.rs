//! Bounded, ordered application frames; heartbeats never await index storage.
use super::SocketCommand;
use crate::AppState;
use axum::extract::ws::Message;
use futures::{Sink, SinkExt, Stream, StreamExt};
use opencoder_core::fleet::{NodeFrame, MAX_FRAME_BYTES, STALE_MS};
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, oneshot};

pub(super) struct Acknowledgement {
    pub(super) message: Message,
    pub(super) done: oneshot::Sender<Result<(), String>>,
}

pub(super) async fn acknowledge(
    tx: &mpsc::Sender<Acknowledgement>,
    message: Message,
) -> anyhow::Result<()> {
    let (done, receipt) = oneshot::channel();
    tokio::time::timeout(Duration::from_secs(5), async {
        tx.send(Acknowledgement { message, done }).await?;
        receipt.await?.map_err(anyhow::Error::msg)
    })
    .await?
}

// Local libsql futures call SQLite synchronously, including its busy timeout.
// A connection awaits each call before starting the next; no report fan-out.
pub(super) async fn storage<T: Send + 'static>(
    operation: impl std::future::Future<Output = anyhow::Result<T>> + Send + 'static,
) -> anyhow::Result<T> {
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || runtime.block_on(operation)).await?
}

async fn send<W>(writer: &mut W, message: Message) -> anyhow::Result<()>
where
    W: Sink<Message> + Unpin,
    W::Error: std::error::Error + Send + Sync + 'static,
{
    tokio::time::timeout(Duration::from_secs(5), writer.send(message)).await??;
    Ok(())
}

pub(super) struct Transport<W, R> {
    pub writer: W,
    pub reader: R,
    pub commands: mpsc::Receiver<SocketCommand>,
    pub acknowledgements: mpsc::Receiver<Acknowledgement>,
    pub incoming: mpsc::Sender<NodeFrame>,
}

impl<W, R> Transport<W, R>
where
    W: Sink<Message> + Unpin,
    W::Error: std::error::Error + Send + Sync + 'static,
    R: Stream<Item = Result<Message, axum::Error>> + Unpin,
{
    pub async fn run(
        self,
        state: Arc<AppState>,
        id: String,
        generation: String,
    ) -> anyhow::Result<()> {
        self.run_with_budget(
            state,
            id,
            generation,
            Duration::from_millis(STALE_MS as u64),
        )
        .await
    }

    async fn run_with_budget(
        mut self,
        state: Arc<AppState>,
        id: String,
        generation: String,
        input_budget: Duration,
    ) -> anyhow::Result<()> {
        let mut pending = None;
        let mut input_deadline = tokio::time::Instant::now() + input_budget;
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(input_deadline) => {
                    anyhow::bail!("node channel input stalled");
                }
                _ = state.lifecycle.channels_retired() => {
                    send(&mut self.writer, Message::Close(None)).await?;
                    return Ok(());
                }
                Some(ack) = self.acknowledgements.recv() => {
                    let result = send(&mut self.writer, ack.message).await;
                    let _ = ack.done.send(result.as_ref().map(|_| ()).map_err(ToString::to_string));
                    result?;
                }
                Some(command) = self.commands.recv() => {
                    let SocketCommand::Frame(frame) = command else {
                        send(&mut self.writer, Message::Close(None)).await?;
                        return Ok(());
                    };
                    let text = serde_json::to_string(frame.as_ref())?;
                    anyhow::ensure!(text.len() <= MAX_FRAME_BYTES, "server frame exceeds limit");
                    send(&mut self.writer, Message::Text(text)).await?;
                }
                permit = self.incoming.reserve(), if pending.is_some() => {
                    permit?.send(pending.take().expect("pending application frame"));
                }
                incoming = self.reader.next(), if pending.is_none() => {
                    let Some(message) = incoming else { return Ok(()); };
                    input_deadline = tokio::time::Instant::now() + input_budget;
                    match message? {
                        Message::Text(text) => {
                            let frame = serde_json::from_str::<NodeFrame>(&text)?;
                            // Input liveness is independent of index storage.
                            // A complete valid frame proves the peer is alive;
                            // it does not promote readiness or apply its data.
                            state.hub.touch(&id, &generation).await;
                            // Keep one pending frame under backpressure. The
                            // writer/ack must keep running if processing waits
                            // for a handoff write while this queue is full.
                            pending = Some(frame);
                        }
                        Message::Ping(data) => {
                            // Liveness does not change load or index_synced.
                            state.hub.touch(&id, &generation).await;
                            send(&mut self.writer, Message::Pong(data)).await?;
                        }
                        Message::Close(_) => return Ok(()),
                        _ => anyhow::bail!("invalid node frame"),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "socket_io/tests.rs"]
mod tests;

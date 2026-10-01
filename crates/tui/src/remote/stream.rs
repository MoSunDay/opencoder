use super::client::ServerClient;
use anyhow::{Context, Result};
use reqwest::Method;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub struct Frame {
    pub seq: Option<i64>,
    pub kind: String,
    pub data: Value,
}

#[derive(Default)]
pub struct Decoder {
    pending: Vec<u8>,
}
impl Decoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Frame>> {
        self.pending.extend_from_slice(bytes);
        anyhow::ensure!(
            self.pending.len() <= 4 * 1024 * 1024,
            "Server SSE frame exceeds 4 MiB"
        );
        let mut frames = Vec::new();
        loop {
            let boundary = self
                .pending
                .windows(2)
                .position(|p| p == b"\n\n")
                .map(|p| (p, 2))
                .or_else(|| {
                    self.pending
                        .windows(4)
                        .position(|p| p == b"\r\n\r\n")
                        .map(|p| (p, 4))
                });
            let Some((end, separator)) = boundary else {
                break;
            };
            let source = String::from_utf8(self.pending.drain(..end + separator).collect())?;
            let mut kind = String::new();
            let mut seq = None;
            let mut data = Vec::new();
            for line in source.lines() {
                let Some((field, value)) = line.split_once(':') else {
                    continue;
                };
                let value = value.strip_prefix(' ').unwrap_or(value);
                match field {
                    "event" => kind = value.into(),
                    "id" => seq = Some(value.parse().context("invalid SSE event ID")?),
                    "data" => data.push(value),
                    _ => {}
                }
            }
            if !kind.is_empty() {
                let data = data.join("\n");
                let data = if kind == "reconnect" {
                    Value::Null
                } else {
                    serde_json::from_str(&data)?
                };
                frames.push(Frame { seq, kind, data });
            }
        }
        Ok(frames)
    }
}

/// This task owns only the subscription. Dropping it never interrupts Server.
pub async fn subscribe(
    client: ServerClient,
    id: String,
    mut cursor: i64,
    tx: mpsc::Sender<Result<Frame>>,
    detach: CancellationToken,
) {
    let mut warned = false;
    loop {
        let attempt = async {
            let mut response = client
                .response(
                    Method::GET,
                    &format!("/api/executions/{id}/events?after={cursor}"),
                    None,
                    true,
                )
                .await?;
            let mut decoder = Decoder::default();
            while let Some(chunk) = response.chunk().await? {
                for mut frame in decoder.push(&chunk)? {
                    if matches!(frame.kind.as_str(), "reconnect" | "stream_end") {
                        return Ok::<(), anyhow::Error>(());
                    }
                    if frame.seq.is_none() && frame.kind == "error" {
                        anyhow::bail!(
                            "{}",
                            frame.data["error"]
                                .as_str()
                                .unwrap_or("Server event subscription failed")
                        );
                    }
                    if frame.seq.is_some_and(|seq| seq <= cursor) {
                        continue;
                    }
                    frame.data =
                        super::transcript::event_data(&client, &id, frame.seq, frame.data).await?;
                    if let Some(seq) = frame.seq {
                        cursor = seq;
                    }
                    tx.send(Ok(frame))
                        .await
                        .map_err(|_| anyhow::anyhow!("UI detached"))?;
                    warned = false;
                }
            }
            Ok(())
        };
        tokio::select! {
            _ = detach.cancelled() => return,
            result = attempt => if let Err(error) = result {
                if !warned {
                    if tx.send(Err(error)).await.is_err() { return; }
                    warned = true;
                }
            }
        }
        tokio::select! {
            _ = detach.cancelled() => return,
            _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {}
        }
    }
}

use super::transport::connect_with_retry;
use crate::retry::{backoff_delay, StreamInterruption, MAX_STREAM_ATTEMPTS};
use crate::{responses::decode::Decoder, sse::SseDecoder, LlmEvent};
use anyhow::{anyhow, Result};
use futures::StreamExt;
use opencoder_core::ProviderState;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;

pub(super) struct Request {
    pub http: reqwest::Client,
    pub url: String,
    pub key: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
    pub idle_timeout: Duration,
}

enum Failure {
    Terminal(anyhow::Error),
    Interrupted(StreamInterruption),
}

pub(super) async fn run(
    req: Request,
    scope: ProviderState,
    tx: mpsc::Sender<LlmEvent>,
) -> Result<()> {
    for attempt in 1..=MAX_STREAM_ATTEMPTS {
        match once(&req, scope.clone(), &tx).await {
            Ok(()) => return Ok(()),
            Err(Failure::Terminal(error)) => return Err(error),
            Err(Failure::Interrupted(reason)) => {
                if attempt == MAX_STREAM_ATTEMPTS {
                    return Err(anyhow!(
                        "Responses stream {reason} after {attempt} attempts"
                    ));
                }
                if tx
                    .send(LlmEvent::Retrying {
                        attempt,
                        max: MAX_STREAM_ATTEMPTS,
                    })
                    .await
                    .is_err()
                {
                    return Ok(());
                }
                tokio::select! { biased; _ = tx.closed() => return Ok(()), _ = backoff_delay(attempt) => {} }
            }
        }
    }
    unreachable!("nonempty retry budget")
}

async fn once(
    req: &Request,
    scope: ProviderState,
    tx: &mpsc::Sender<LlmEvent>,
) -> Result<(), Failure> {
    let Some(resp) = connect_with_retry(&req.http, &req.url, &req.key, &req.headers, &req.body, tx)
        .await
        .map_err(Failure::Terminal)?
    else {
        return Ok(());
    };
    let is_json = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| h.split(';').next() == Some("application/json"));
    let mut decoder = Decoder::new(scope);
    if is_json {
        let value = tokio::select! {
            biased;
            _ = tx.closed() => return Ok(()),
            result = tokio::time::timeout(req.idle_timeout, resp.json::<Value>()) => match result {
                Err(_) => return Err(Failure::Interrupted(StreamInterruption::IdleTimeout)),
                Ok(Err(e)) if e.is_body() || e.is_timeout() => return Err(Failure::Interrupted(StreamInterruption::ChunkError)),
                Ok(result) => result.map_err(|e| Failure::Terminal(anyhow!("invalid JSON Response: {e}")))?,
            }
        };
        deliver(decoder.push(value).map_err(Failure::Terminal)?, tx).await;
        return if decoder.completed {
            Ok(())
        } else {
            Err(Failure::Terminal(anyhow!(
                "JSON body is not a completed Response"
            )))
        };
    }
    let mut bytes = resp.bytes_stream();
    let mut sse = SseDecoder::new();
    let mut deadline = tokio::time::Instant::now() + req.idle_timeout;
    loop {
        let chunk = tokio::select! {
            biased;
            _ = tx.closed() => return Ok(()),
            _ = tokio::time::sleep_until(deadline) => return Err(Failure::Interrupted(StreamInterruption::IdleTimeout)),
            chunk = bytes.next() => chunk,
        };
        let frames = match chunk {
            Some(Ok(chunk)) => {
                sse.push(&chunk);
                sse.drain()
            }
            Some(Err(_)) => return Err(Failure::Interrupted(StreamInterruption::ChunkError)),
            None => {
                consume(sse.flush_remaining(), &mut decoder, tx).await?;
                return if decoder.completed {
                    Ok(())
                } else {
                    Err(Failure::Interrupted(StreamInterruption::Truncated))
                };
            }
        };
        let progressed = !frames.is_empty();
        consume(frames, &mut decoder, tx).await?;
        if decoder.completed || tx.is_closed() {
            return Ok(());
        }
        if progressed {
            deadline = tokio::time::Instant::now() + req.idle_timeout;
        }
    }
}

async fn consume(
    frames: Vec<String>,
    decoder: &mut Decoder,
    tx: &mpsc::Sender<LlmEvent>,
) -> Result<(), Failure> {
    for frame in frames {
        let value = serde_json::from_str(&frame)
            .map_err(|e| Failure::Terminal(anyhow!("invalid Responses SSE JSON: {e}")))?;
        deliver(decoder.push(value).map_err(Failure::Terminal)?, tx).await;
        if decoder.completed || tx.is_closed() {
            break;
        }
    }
    Ok(())
}

async fn deliver(events: Vec<LlmEvent>, tx: &mpsc::Sender<LlmEvent>) {
    for event in events {
        if tx.send(event).await.is_err() {
            break;
        }
    }
}

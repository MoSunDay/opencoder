use super::{chat_events::handle_event, transport::connect_with_retry};
use crate::retry::{
    backoff_delay, should_retry_stream_interruption, StreamInterruption, MAX_STREAM_ATTEMPTS,
};
use crate::{
    event::{LlmEvent, Usage},
    sse::SseDecoder,
    tool_call::ToolAccumulator,
};
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::warn;

enum OnceError {
    Connect(anyhow::Error),
    Interrupted { reason: StreamInterruption },
}

/// Drive a chat completion to completion, retrying mid-stream interruptions
/// (chunk read errors, truncated streams, idle stalls) up to
/// [`MAX_STREAM_ATTEMPTS`] times. The pre-stream connection loop
/// (`connect_with_retry`) runs on every attempt; mid-stream retries reset all
/// per-attempt state (text/tool/usage buffers) so a retried response is
/// regenerated cleanly — the persisted text always comes from a single frame's
/// `Completed`, never stitched across attempts.
pub(super) async fn run_stream(
    client: reqwest::Client,
    url: String,
    key: String,
    headers: Vec<(String, String)>,
    body: Value,
    tx: mpsc::Sender<LlmEvent>,
    idle_timeout: Duration,
) -> Result<()> {
    let mut attempt: u8 = 0;
    loop {
        attempt = attempt.saturating_add(1);
        match run_stream_once(&client, &url, &key, &headers, &body, &tx, idle_timeout).await {
            // `Completed` already emitted to `tx`.
            Ok(()) => return Ok(()),
            Err(OnceError::Connect(e)) => {
                // Connection-level retries already exhausted inside
                // `connect_with_retry`; surface as a terminal error.
                // Don't emit LlmEvent::Error here — the spawn task in
                // chat_stream catches the returned Err and emits it.
                return Err(e);
            }
            Err(OnceError::Interrupted { reason }) => {
                let can_retry =
                    should_retry_stream_interruption(reason) && attempt < MAX_STREAM_ATTEMPTS;
                if !can_retry {
                    return Err(anyhow!("stream {reason} after {attempt} attempts"));
                }
                // Retry: tell consumers to discard accumulated deltas, back off,
                // then reconnect for a fresh attempt.
                warn!(attempt, reason = ?reason, "mid-stream interruption, retrying");
                let _ = tx
                    .send(LlmEvent::Retrying {
                        attempt,
                        max: MAX_STREAM_ATTEMPTS,
                    })
                    .await;
                tokio::select! {
                    biased;
                    _ = tx.closed() => return Ok(()),
                    _ = backoff_delay(attempt) => {}
                }
            }
        }
    }
}

/// Run a single stream attempt end to end. On success emits exactly one
/// `Completed`; on interruption returns the partial output for the wrapper.
async fn run_stream_once(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    headers: &[(String, String)],
    body: &Value,
    tx: &mpsc::Sender<LlmEvent>,
    idle_timeout: Duration,
) -> Result<(), OnceError> {
    let resp = match connect_with_retry(client, url, key, headers, body, tx)
        .await
        .map_err(OnceError::Connect)?
    {
        Some(resp) => resp,
        // Consumer dropped during pre-stream retries — stop cleanly; there is
        // no one left to deliver events to.
        None => return Ok(()),
    };
    let mut stream = resp.bytes_stream();
    let mut decoder = SseDecoder::new();
    let mut tools = ToolAccumulator::default();
    let mut usage: Option<Usage> = None;
    let mut finished = false;
    let mut text_buf = String::new();
    // Cross-frame guard: set once any frame streamed reasoning via delta (or
    // the `choice.message` fallback). Suppresses duplicate fallback emission.
    let mut streamed_reasoning = false;
    // Event-level idle watchdog: reset whenever at least one SSE data frame is
    // decoded. A keep-alive-only connection delivers bytes (so the HTTP
    // read_timeout never trips) but no data frames, so this elapsed check —
    // not the byte-level timeout — is what catches it.
    let mut last_event_at = Instant::now();

    use futures::StreamExt;
    loop {
        let chunk = tokio::select! {
            biased;
            _ = tx.closed() => return Ok(()),
            chunk_opt = stream.next() => match chunk_opt {
                Some(chunk) => chunk,
                None => break,
            },
            _ = tokio::time::sleep(idle_timeout) => {
                if finished {
                    let tool_calls = tools.finish_all().map_err(OnceError::Connect)?;
                    let _ = tx
                        .send(LlmEvent::Completed {
                            text: std::mem::take(&mut text_buf),
                            tool_calls,
                            usage,
                        })
                        .await;
                    return Ok(());
                }
                return Err(OnceError::Interrupted {
                    reason: StreamInterruption::IdleTimeout,
                });
            }
        };
        let bytes = match chunk {
            Ok(b) => b,
            Err(e) => {
                if finished {
                    let tool_calls = tools.finish_all().map_err(OnceError::Connect)?;
                    let _ = tx
                        .send(LlmEvent::Completed {
                            text: std::mem::take(&mut text_buf),
                            tool_calls,
                            usage,
                        })
                        .await;
                    return Ok(());
                }
                warn!(error = %e, "stream chunk read error");
                return Err(OnceError::Interrupted {
                    reason: StreamInterruption::ChunkError,
                });
            }
        };
        decoder.push(&bytes);
        let frames = decoder.drain();
        let had_frames = !frames.is_empty();
        for data in frames {
            if tx.is_closed() {
                return Ok(());
            }
            {
                let parsed = serde_json::from_str(&data)
                    .map_err(|e| OnceError::Connect(anyhow!("invalid SSE JSON: {e}")))?;
                handle_event(
                    &parsed,
                    &mut tools,
                    &mut usage,
                    &mut finished,
                    &mut text_buf,
                    &mut streamed_reasoning,
                    tx,
                )
                .await
                .map_err(OnceError::Connect)?;
            }
        }
        // Stamp the watchdog AFTER delivering frames to the consumer. Stamping
        // before the `tx.send` loop would make the elapsed check include time
        // blocked on a full channel (slow-consumer back-pressure), producing
        // spurious idle-timeout retries that measure consumer lag rather than
        // upstream silence.
        if had_frames {
            last_event_at = Instant::now();
        }
        if last_event_at.elapsed() >= idle_timeout {
            if finished {
                let tool_calls = tools.finish_all().map_err(OnceError::Connect)?;
                let _ = tx
                    .send(LlmEvent::Completed {
                        text: std::mem::take(&mut text_buf),
                        tool_calls,
                        usage,
                    })
                    .await;
                return Ok(());
            }
            return Err(OnceError::Interrupted {
                reason: StreamInterruption::IdleTimeout,
            });
        }
    }
    // Stream ended — flush any buffered partial frame the decoder still holds.
    for data in decoder.flush_remaining() {
        {
            let parsed = serde_json::from_str(&data)
                .map_err(|e| OnceError::Connect(anyhow!("invalid SSE JSON: {e}")))?;
            handle_event(
                &parsed,
                &mut tools,
                &mut usage,
                &mut finished,
                &mut text_buf,
                &mut streamed_reasoning,
                tx,
            )
            .await
            .map_err(OnceError::Connect)?;
        }
    }

    if finished {
        let tool_calls = tools.finish_all().map_err(OnceError::Connect)?;
        let _ = tx
            .send(LlmEvent::Completed {
                text: text_buf,
                tool_calls,
                usage,
            })
            .await;
        Ok(())
    } else {
        // No `finish_reason` seen — the stream was truncated.
        Err(OnceError::Interrupted {
            reason: StreamInterruption::Truncated,
        })
    }
}

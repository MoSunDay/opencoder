use super::build_header_map;
use crate::retry::{
    backoff_delay, backoff_duration, retry_decision, retry_delay, AttemptOutcome, RetryDecision,
    MAX_ATTEMPTS,
};
use crate::{event::LlmEvent, http_date::parse_http_date_to_secs};
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::warn;

/// Build and send a single chat request, returning the raw response (status
/// unchecked). The caller decides retryability.
async fn send_request(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    headers: &[(String, String)],
    body: &Value,
) -> Result<reqwest::Response> {
    let header_map = build_header_map(key, headers)?;
    client
        .post(url)
        .headers(header_map)
        .json(body)
        .send()
        .await
        .context("send chat request")
}

/// Retry the request up to `MAX_ATTEMPTS` times, but only before any streamed
/// bytes are produced. Emits `LlmEvent::Retrying` before each backoff so the UI
/// can surface "↻ retry n/5". Non-retryable HTTP errors (4xx other than the
/// whitelisted set) fail immediately. Every retry-vs-fail-vs-done decision
/// delegates to the pure `retry_decision` policy.
pub(super) async fn connect_with_retry(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    headers: &[(String, String)],
    body: &Value,
    tx: &mpsc::Sender<LlmEvent>,
) -> Result<Option<reqwest::Response>> {
    let mut attempt: u8 = 0;
    loop {
        // The consumer (the `rx` half of the channel) may have been dropped
        // while we slept between retries. There is no point issuing another
        // request — bail out cleanly instead of looping until exhaustion.
        if tx.is_closed() {
            return Ok(None);
        }
        attempt = attempt.saturating_add(1);
        let send_result = tokio::select! {
            biased;
            _ = tx.closed() => return Ok(None),
            r = send_request(client, url, key, headers, body) => r,
        };
        match send_result {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    return Ok(Some(resp));
                }
                let outcome = AttemptOutcome::from_status(status);
                if retry_decision(outcome, attempt, MAX_ATTEMPTS) == RetryDecision::Fail {
                    let text = tokio::select! {
                        biased;
                        _ = tx.closed() => return Ok(None),
                        body = resp.text() => body.unwrap_or_default(),
                    };
                    // Preserve the two distinct error messages: a fast fail for
                    // non-retryable statuses, an exhaustion message otherwise.
                    return Err(if outcome == AttemptOutcome::NonRetryableError {
                        anyhow!("http {status}: {}", truncate(&text, 800))
                    } else {
                        anyhow!(
                            "http {status} after {attempt} attempts: {}",
                            truncate(&text, 800)
                        )
                    });
                }
                warn!(attempt, status = status.as_u16(), "retryable HTTP status");
                // Read `Retry-After` and drain the body BEFORE sleeping: an
                // unread body keeps the connection tied up (preventing pool
                // reuse), and the header may demand a longer wait than backoff
                // (bounded by `RETRY_AFTER_MAX_SECS` so a hostile server
                // cannot stall the session for a day).
                let retry_after = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok().or_else(|| parse_http_date_to_secs(s)));
                tokio::select! {
                    biased;
                    _ = tx.closed() => return Ok(None),
                    _ = resp.text() => {}
                }
                let _ = tx
                    .send(LlmEvent::Retrying {
                        attempt,
                        max: MAX_ATTEMPTS,
                    })
                    .await;
                let delay = retry_delay(retry_after, backoff_duration(attempt));
                tokio::select! {
                    biased;
                    _ = tx.closed() => return Ok(None),
                    _ = tokio::time::sleep(delay) => {}
                }
            }
            Err(e) => {
                // Network/transport error — treat as transient.
                if retry_decision(AttemptOutcome::RetryableError, attempt, MAX_ATTEMPTS)
                    == RetryDecision::Fail
                {
                    return Err(
                        e.context(format!("send chat request failed after {attempt} attempts"))
                    );
                }
                warn!(attempt, error = %e, "send error, will retry");
                let _ = tx
                    .send(LlmEvent::Retrying {
                        attempt,
                        max: MAX_ATTEMPTS,
                    })
                    .await;
                tokio::select! {
                    biased;
                    _ = tx.closed() => return Ok(None),
                    _ = backoff_delay(attempt) => {}
                }
            }
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(n).collect();
        format!("{truncated}...")
    }
}

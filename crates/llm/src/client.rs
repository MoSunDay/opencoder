use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::event::{LlmEvent, Usage};
use crate::request::ChatRequest;
use crate::sse::SseDecoder;
use crate::stream::ChatStream;
use crate::tool_call::ToolAccumulator;

#[derive(Debug, Clone)]
pub struct ChatParams {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u64>,
}

#[derive(Clone)]
pub struct ChatClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

/// Default per-read idle timeout (5 minutes). A read that stalls for this
/// long without receiving any bytes is aborted; a stream that keeps
/// delivering data resets the timer on every chunk and is never interrupted.
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(300);

impl ChatClient {
    pub fn new(base_url: &str, api_key: &str) -> Result<Self> {
        Self::new_with_read_timeout(base_url, api_key, DEFAULT_READ_TIMEOUT)
    }

    /// Construct a client with a custom per-read idle timeout. Useful for
    /// tests that need a short stall window.
    pub fn new_with_read_timeout(
        base_url: &str,
        api_key: &str,
        read_timeout: Duration,
    ) -> Result<Self> {
        let http = reqwest::Client::builder()
            .read_timeout(read_timeout)
            .connect_timeout(Duration::from_secs(30))
            .build()
            .context("build http client")?;
        Ok(ChatClient {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
        })
    }

    pub fn chat_stream(&self, req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
        let (tx, rx) = mpsc::channel::<LlmEvent>(128);
        let url = format!("{}/chat/completions", self.base_url);
        let body = req.to_body();
        let client = self.http.clone();
        let key = self.api_key.clone();

        tokio::spawn(async move {
            if let Err(e) = run_stream(client, url, key, body, tx.clone()).await {
                let _ = tx
                    .send(LlmEvent::Error(format!("stream failed: {e:#}")))
                    .await;
            }
        });
        Ok(rx)
    }
}

impl ChatStream for ChatClient {
    fn chat_stream(&self, req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
        ChatClient::chat_stream(self, req)
    }
    fn backend(&self) -> &'static str {
        "openai"
    }
}

async fn run_stream(
    client: reqwest::Client,
    url: String,
    key: String,
    body: Value,
    tx: mpsc::Sender<LlmEvent>,
) -> Result<()> {
    // Pre-stream retry loop: retries ONLY connection + initial HTTP status,
    // never mid-stream. This guarantees partial streamed output can never be
    // duplicated by a retry (a retry only happens when NO bytes have been
    // emitted to the consumer yet).
    let resp = connect_with_retry(&client, &url, &key, &body, &tx).await?;

    let mut stream = resp.bytes_stream();
    let mut decoder = SseDecoder::new();
    let mut tools = ToolAccumulator::default();
    let mut usage: Option<Usage> = None;
    let mut finished = false;
    let mut text_buf = String::new();

    use futures::StreamExt;
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.context("read stream chunk")?;
        decoder.push(&bytes);
        for data in decoder.drain() {
            if tx.is_closed() {
                return Ok(());
            }
            let parsed = match crate::sse::parse_chunk(&data) {
                Some(v) => v,
                None => continue,
            };
            handle_event(
                &parsed,
                &mut tools,
                &mut usage,
                &mut finished,
                &mut text_buf,
                &tx,
            )
            .await?;
        }
    }
    for data in decoder.flush_remaining() {
        if let Some(parsed) = crate::sse::parse_chunk(&data) {
            handle_event(
                &parsed,
                &mut tools,
                &mut usage,
                &mut finished,
                &mut text_buf,
                &tx,
            )
            .await?;
        }
    }

    let tool_calls = tools.finish_all().unwrap_or_default();
    let _ = tx
        .send(LlmEvent::Completed {
            text: text_buf,
            tool_calls,
            usage,
        })
        .await;
    Ok(())
}

/// Total request attempts (1 initial + 4 retries).
const MAX_ATTEMPTS: u8 = 5;
/// Base backoff in ms; actual delay is `BASE_BACKOFF_MS * 2^(attempt-1)` plus
/// up to 250 ms jitter, giving roughly 0.5/1/2/4/8 s between attempts.
const BASE_BACKOFF_MS: u64 = 500;

/// Whether an HTTP status is transient enough to warrant a retry. Network/send
/// errors (no status) are always retried; only these status codes qualify.
fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

/// Classification of a single send attempt's outcome, abstracted away from
/// `reqwest` so the retry decision can be unit-tested without HTTP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttemptOutcome {
    /// 2xx — the request succeeded; stop and consume the response.
    Success,
    /// A transient failure worth retrying (whitelisted status, or a network/
    /// transport error with no status at all).
    RetryableError,
    /// A permanent failure (4xx other than the whitelist) — fail immediately.
    NonRetryableError,
}

impl AttemptOutcome {
    /// Classify an HTTP response status into an attempt outcome.
    fn from_status(status: reqwest::StatusCode) -> Self {
        if status.is_success() {
            Self::Success
        } else if is_retryable_status(status) {
            Self::RetryableError
        } else {
            Self::NonRetryableError
        }
    }
}

/// What the retry loop should do after observing an attempt's outcome, given
/// the current 1-based `attempt` number and the `max` attempts allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryDecision {
    /// Request succeeded — stop and return the response.
    Done,
    /// Transient failure, attempts remaining — emit `Retrying` and back off.
    Retry,
    /// Permanent failure OR retries exhausted — stop with an error.
    Fail,
}

/// Pure retry policy with no I/O, so the loop's boundary logic (the part prone
/// to off-by-one errors) is exhaustively unit-testable. `connect_with_retry`
/// delegates every retry-vs-fail-vs-done decision here.
fn retry_decision(outcome: AttemptOutcome, attempt: u8, max: u8) -> RetryDecision {
    match outcome {
        AttemptOutcome::Success => RetryDecision::Done,
        AttemptOutcome::NonRetryableError => RetryDecision::Fail,
        AttemptOutcome::RetryableError => {
            if attempt >= max {
                RetryDecision::Fail
            } else {
                RetryDecision::Retry
            }
        }
    }
}

/// Build and send a single chat request, returning the raw response (status
/// unchecked). The caller decides retryability.
async fn send_request(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    body: &Value,
) -> Result<reqwest::Response> {
    client
        .post(url)
        .bearer_auth(key)
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .json(body)
        .send()
        .await
        .context("send chat request")
}

/// Exponential backoff delay (ms) for the given 1-based `attempt`, BEFORE
/// jitter: `BASE_BACKOFF_MS * 2^(attempt-1)` → 500/1000/2000/4000/8000 ms.
/// Extracted as a pure function so the growth curve is unit-testable.
fn backoff_millis(attempt: u8) -> u64 {
    BASE_BACKOFF_MS.saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1) as u32))
}

/// Exponential backoff for the given 1-based `attempt`, with up to 250 ms of
/// jitter derived from the wall clock (no `rand` dependency). Jitter avoids
/// synchronized retry bursts when many clients share a flaky endpoint.
async fn backoff_delay(attempt: u8) {
    let exp = backoff_millis(attempt);
    let jitter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| (d.subsec_nanos() as u64) % 250)
        .unwrap_or(0);
    tokio::time::sleep(Duration::from_millis(exp + jitter)).await;
}

/// Retry the request up to `MAX_ATTEMPTS` times, but only before any streamed
/// bytes are produced. Emits `LlmEvent::Retrying` before each backoff so the UI
/// can surface "↻ retry n/5". Non-retryable HTTP errors (4xx other than the
/// whitelisted set) fail immediately. Every retry-vs-fail-vs-done decision
/// delegates to the pure `retry_decision` policy.
async fn connect_with_retry(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    body: &Value,
    tx: &mpsc::Sender<LlmEvent>,
) -> Result<reqwest::Response> {
    let mut attempt: u8 = 0;
    loop {
        attempt = attempt.saturating_add(1);
        match send_request(client, url, key, body).await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    return Ok(resp);
                }
                let outcome = AttemptOutcome::from_status(status);
                if retry_decision(outcome, attempt, MAX_ATTEMPTS) == RetryDecision::Fail {
                    let text = resp.text().await.unwrap_or_default();
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
                let _ = tx
                    .send(LlmEvent::Retrying {
                        attempt,
                        max: MAX_ATTEMPTS,
                    })
                    .await;
                backoff_delay(attempt).await;
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
                backoff_delay(attempt).await;
            }
        }
    }
}

async fn handle_event(
    parsed: &Value,
    tools: &mut ToolAccumulator,
    usage: &mut Option<Usage>,
    finished: &mut bool,
    text_buf: &mut String,
    tx: &mpsc::Sender<LlmEvent>,
) -> Result<()> {
    if let Some(u) = parsed.get("usage") {
        *usage = Some(parse_usage(u));
    }
    let choices = match parsed.get("choices").and_then(|c| c.as_array()) {
        Some(c) => c,
        None => return Ok(()),
    };
    for choice in choices {
        if let Some(delta) = choice.get("delta") {
            emit_delta(delta, tools, text_buf, tx).await?;
        }
        if let Some(fr) = choice.get("finish_reason").and_then(|v| v.as_str()) {
            *finished = true;
            if matches!(fr, "length" | "content_filter") {
                // Benign in common cases (e.g. hitting a max-tokens budget on a
                // long but otherwise healthy turn). Demoted from `warn!` so it
                // does not surface as noise; full context still reaches the log
                // file at debug level.
                debug!(finish_reason = fr, "stream finished early");
            }
        }
    }
    Ok(())
}

async fn emit_delta(
    delta: &Value,
    tools: &mut ToolAccumulator,
    text_buf: &mut String,
    tx: &mpsc::Sender<LlmEvent>,
) -> Result<()> {
    if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
        if !content.is_empty() {
            text_buf.push_str(content);
            let _ = tx.send(LlmEvent::TextDelta(content.to_string())).await;
        }
    }
    if let Some(reasoning) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
        if !reasoning.is_empty() {
            let _ = tx
                .send(LlmEvent::ReasoningDelta(reasoning.to_string()))
                .await;
        }
    }
    if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
        for tc in tool_calls {
            let index = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let id = tc.get("id").and_then(|v| v.as_str());
            let name = tc
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str());
            let args = tc
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(|v| v.as_str());
            for ev in tools.apply(index, id, name, args) {
                let _ = tx.send(ev).await;
            }
        }
    }
    Ok(())
}

fn parse_usage(u: &Value) -> Usage {
    Usage {
        input_tokens: u
            .get("prompt_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or_default(),
        output_tokens: u
            .get("completion_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or_default(),
        total_tokens: u
            .get("total_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or_default(),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard: the default read timeout must stay at 300 s (5 min).
    /// Accidentally changing it (e.g. to 30 s) would break long-running
    /// streaming turns from models that pause between chunks.
    #[test]
    fn default_read_timeout_is_300s() {
        assert_eq!(DEFAULT_READ_TIMEOUT, Duration::from_secs(300));
    }

    #[test]
    fn retryable_status_whitelist() {
        // Transient — retried.
        for code in [408, 425, 429, 500, 502, 503, 504] {
            assert!(
                is_retryable_status(reqwest::StatusCode::from_u16(code).unwrap()),
                "{code} should be retryable"
            );
        }
    }

    #[test]
    fn non_retryable_status_fails_fast() {
        // Auth / bad-request / not-found / redirect are NOT retried.
        for code in [400, 401, 403, 404, 422] {
            assert!(
                !is_retryable_status(reqwest::StatusCode::from_u16(code).unwrap()),
                "{code} should fail fast"
            );
        }
        // 200 is success, not "retryable" (handled separately).
        assert!(!is_retryable_status(reqwest::StatusCode::OK));
    }

    /// The backoff curve doubles each attempt: 0.5/1/2/4/8 s for attempts
    /// 1–5. This exercises the actual `backoff_millis` production function
    /// (the `saturating_mul`/`saturating_pow` math), not just constants.
    #[test]
    fn backoff_millis_doubles_each_attempt() {
        assert_eq!(backoff_millis(1), 500);
        assert_eq!(backoff_millis(2), 1000);
        assert_eq!(backoff_millis(3), 2000);
        assert_eq!(backoff_millis(4), 4000);
        assert_eq!(backoff_millis(5), 8000);
        assert_eq!(MAX_ATTEMPTS, 5);
    }

    /// `AttemptOutcome::from_status` must classify success/retryable/fail the
    /// same way the production loop does (it is the loop's classifier).
    #[test]
    fn attempt_outcome_classifies_status() {
        use reqwest::StatusCode;
        assert_eq!(
            AttemptOutcome::from_status(StatusCode::OK),
            AttemptOutcome::Success
        );
        for code in [408, 425, 429, 500, 502, 503, 504] {
            assert_eq!(
                AttemptOutcome::from_status(StatusCode::from_u16(code).unwrap()),
                AttemptOutcome::RetryableError,
                "{code} should classify as retryable"
            );
        }
        for code in [400, 401, 403, 404, 422] {
            assert_eq!(
                AttemptOutcome::from_status(StatusCode::from_u16(code).unwrap()),
                AttemptOutcome::NonRetryableError,
                "{code} should classify as non-retryable"
            );
        }
    }

    /// Success always stops immediately, regardless of attempt number.
    #[test]
    fn retry_decision_success_stops() {
        assert_eq!(
            retry_decision(AttemptOutcome::Success, 1, 5),
            RetryDecision::Done
        );
        assert_eq!(
            retry_decision(AttemptOutcome::Success, 5, 5),
            RetryDecision::Done
        );
    }

    /// A non-retryable error fails FAST on every attempt — never retries.
    #[test]
    fn retry_decision_non_retryable_fails_fast() {
        for attempt in 1..=5u8 {
            assert_eq!(
                retry_decision(AttemptOutcome::NonRetryableError, attempt, 5),
                RetryDecision::Fail,
                "non-retryable must fail fast at attempt {attempt}"
            );
        }
    }

    /// A retryable error retries while attempts remain and FAILS exactly when
    /// attempt == max (no sixth attempt). This is the off-by-one canary.
    #[test]
    fn retry_decision_retryable_retries_then_fails_at_max() {
        for attempt in 1..=4u8 {
            assert_eq!(
                retry_decision(AttemptOutcome::RetryableError, attempt, 5),
                RetryDecision::Retry,
                "attempt {attempt} (< max=5) should retry"
            );
        }
        // attempt == max: the last allowed attempt already happened — fail.
        assert_eq!(
            retry_decision(AttemptOutcome::RetryableError, 5, 5),
            RetryDecision::Fail,
            "attempt == max must fail (no attempt beyond max)"
        );
    }

    /// Replay a full retry-then-recover sequence through the policy: 2
    /// retryable failures then success. Verifies the loop's decision stream
    /// without needing HTTP — exactly what `connect_with_retry` produces when
    /// the endpoint recovers on attempt 3.
    #[test]
    fn retry_decision_sequence_recover_on_third_attempt() {
        let max = 5u8;
        // Attempt 1: retryable → retry.
        assert_eq!(
            retry_decision(AttemptOutcome::RetryableError, 1, max),
            RetryDecision::Retry
        );
        // Attempt 2: retryable → retry.
        assert_eq!(
            retry_decision(AttemptOutcome::RetryableError, 2, max),
            RetryDecision::Retry
        );
        // Attempt 3: success → done.
        assert_eq!(
            retry_decision(AttemptOutcome::Success, 3, max),
            RetryDecision::Done
        );
    }
}

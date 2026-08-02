use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::event::{LlmEvent, Usage};
use crate::request::ChatRequest;
use crate::retry::{
    backoff_delay, retry_decision, should_retry_stream_interruption, AttemptOutcome,
    RetryDecision, StreamInterruption, MAX_ATTEMPTS, MAX_STREAM_ATTEMPTS,
};
use crate::sse::SseDecoder;
use crate::stream::ChatStream;
use crate::tool_call::{CompletedToolCall, ToolAccumulator};

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
    headers: Vec<(String, String)>,
    /// Event-level idle watchdog: if no decoded SSE event arrives within this
    /// window, the stream is treated as interrupted (and retried). Independent
    /// of the HTTP client's byte-level `read_timeout`, which catches total
    /// stalls; this catches a connection dribbling keep-alive heartbeats with
    /// no content.
    idle_timeout: Duration,
}

/// Default per-read idle timeout (10 minutes). A read that stalls for this
/// long without receiving any bytes is aborted; a stream that keeps
/// delivering data resets the timer on every chunk and is never interrupted.
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(600);

impl ChatClient {
    /// Construct a client. `proxy` is an optional explicit proxy URL (e.g.
    /// `socks5://host:port`); when `None`, the proxy is resolved from
    /// `OPENCODER_PROXY` / `ALL_PROXY` / `HTTPS_PROXY` / `HTTP_PROXY`. Loopback
    /// hosts always bypass the proxy.
    pub fn new(
        base_url: &str,
        api_key: &str,
        headers: &[(String, String)],
        proxy: Option<&str>,
    ) -> Result<Self> {
        Self::new_with_read_timeout(base_url, api_key, headers, DEFAULT_READ_TIMEOUT, proxy)
    }

    /// Construct a client with a custom per-read timeout. The same value
    /// governs both the HTTP client's byte-level `read_timeout` and the
    /// event-level idle watchdog, so a configured `stream_idle_timeout` flows
    /// through both layers consistently. See [`Self::new`] for proxy semantics.
    pub fn new_with_read_timeout(
        base_url: &str,
        api_key: &str,
        headers: &[(String, String)],
        read_timeout: Duration,
        proxy: Option<&str>,
    ) -> Result<Self> {
        let http = opencoder_core::net::build_http_client_with_read_timeout(proxy, read_timeout)?;
        Ok(ChatClient {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            headers: headers.to_vec(),
            idle_timeout: read_timeout,
        })
    }

    pub fn chat_stream(&self, req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
        let (tx, rx) = mpsc::channel::<LlmEvent>(128);
        let url = format!("{}/chat/completions", self.base_url);
        let body = req.to_body();
        let client = self.http.clone();
        let key = self.api_key.clone();
        let headers = self.headers.clone();
        let idle_timeout = self.idle_timeout;

        tokio::spawn(async move {
            if let Err(e) = run_stream(client, url, key, headers, body, tx.clone(), idle_timeout)
                .await
            {
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
}

/// Accumulated output from one stream attempt, returned to the retry wrapper so
/// it can degrade sensibly when the retry budget is exhausted.
struct StreamOutcome {
    text: String,
    tool_calls: Vec<CompletedToolCall>,
    usage: Option<Usage>,
}

/// Snapshot whatever a single attempt accumulated, for degradation on
/// exhaustion. `tools.finish_all()` finalizes in-progress tool calls.
fn snapshot(text: String, tools: &mut ToolAccumulator, usage: Option<Usage>) -> StreamOutcome {
    StreamOutcome {
        text,
        tool_calls: tools.finish_all().unwrap_or_default(),
        usage,
    }
}

/// Outcome of a single stream attempt.
enum OnceError {
    /// Pre-stream connect failed (`connect_with_retry` exhausted its budget).
    Connect(anyhow::Error),
    /// The stream was interrupted mid-flight; `partial` holds whatever was
    /// accumulated so far so the wrapper can decide to retry or degrade.
    Interrupted {
        reason: StreamInterruption,
        partial: StreamOutcome,
    },
}

/// Drive a chat completion to completion, retrying mid-stream interruptions
/// (chunk read errors, truncated streams, idle stalls) up to
/// [`MAX_STREAM_ATTEMPTS`] times. The pre-stream connection loop
/// (`connect_with_retry`) runs on every attempt; mid-stream retries reset all
/// per-attempt state (text/tool/usage buffers) so a retried response is
/// regenerated cleanly — the persisted text always comes from a single frame's
/// `Completed`, never stitched across attempts.
async fn run_stream(
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
                let _ = tx
                    .send(LlmEvent::Error(format!("stream failed: {e:#}")))
                    .await;
                return Err(e);
            }
            Err(OnceError::Interrupted { reason, partial }) => {
                let can_retry =
                    should_retry_stream_interruption(reason) && attempt < MAX_STREAM_ATTEMPTS;
                if !can_retry {
                    // Budget exhausted — degrade by reason so no data is lost
                    // unnecessarily. A truncated stream has usable partial text,
                    // so deliver it as a best-effort `Completed`; chunk errors and
                    // idle stalls produce no coherent text, so they surface as an
                    // `Error`.
                    match reason {
                        StreamInterruption::Truncated => {
                            warn!(
                                attempts = attempt,
                                "stream truncated; delivering partial output after retries"
                            );
                            let _ = tx
                                .send(LlmEvent::Completed {
                                    text: partial.text,
                                    tool_calls: partial.tool_calls,
                                    usage: partial.usage,
                                })
                                .await;
                            return Ok(());
                        }
                        StreamInterruption::ChunkError | StreamInterruption::IdleTimeout => {
                            let kind = match reason {
                                StreamInterruption::ChunkError => "chunk read error",
                                StreamInterruption::IdleTimeout => "idle timeout",
                                StreamInterruption::Truncated => unreachable!(),
                            };
                            let msg = format!("stream failed: {kind} after {attempt} attempts");
                            let _ = tx.send(LlmEvent::Error(msg.clone())).await;
                            return Err(anyhow!("{msg}"));
                        }
                    }
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
                backoff_delay(attempt).await;
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
    let resp = connect_with_retry(client, url, key, headers, body, tx)
        .await
        .map_err(OnceError::Connect)?;
    let mut stream = resp.bytes_stream();
    let mut decoder = SseDecoder::new();
    let mut tools = ToolAccumulator::default();
    let mut usage: Option<Usage> = None;
    let mut finished = false;
    let mut text_buf = String::new();
    // Event-level idle watchdog: reset whenever at least one SSE data frame is
    // decoded. A keep-alive-only connection delivers bytes (so the HTTP
    // read_timeout never trips) but no data frames, so this elapsed check —
    // not the byte-level timeout — is what catches it.
    let mut last_event_at = Instant::now();

    use futures::StreamExt;
    while let Some(chunk) = stream.next().await {
        let bytes = match chunk {
            Ok(b) => b,
            Err(e) => {
                warn!(error = %e, "stream chunk read error");
                return Err(OnceError::Interrupted {
                    reason: StreamInterruption::ChunkError,
                    partial: snapshot(text_buf, &mut tools, usage),
                });
            }
        };
        decoder.push(&bytes);
        let frames = decoder.drain();
        if !frames.is_empty() {
            last_event_at = Instant::now();
        }
        for data in frames {
            if tx.is_closed() {
                return Ok(());
            }
            if let Some(parsed) = crate::sse::parse_chunk(&data) {
                handle_event(&parsed, &mut tools, &mut usage, &mut finished, &mut text_buf, tx)
                    .await
                    .map_err(OnceError::Connect)?;
            }
        }
        if last_event_at.elapsed() >= idle_timeout {
            return Err(OnceError::Interrupted {
                reason: StreamInterruption::IdleTimeout,
                partial: snapshot(text_buf, &mut tools, usage),
            });
        }
    }
    // Stream ended — flush any buffered partial frame the decoder still holds.
    let mut flushed_any = false;
    for data in decoder.flush_remaining() {
        if let Some(parsed) = crate::sse::parse_chunk(&data) {
            handle_event(&parsed, &mut tools, &mut usage, &mut finished, &mut text_buf, tx)
                .await
                .map_err(OnceError::Connect)?;
            flushed_any = true;
        }
    }
    let _ = flushed_any;

    if finished {
        let tool_calls = tools.finish_all().unwrap_or_default();
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
            partial: snapshot(text_buf, &mut tools, usage),
        })
    }
}

/// Build and send a single chat request, returning the raw response (status
/// unchecked). The caller decides retryability.
async fn send_request(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    headers: &[(String, String)],
    body: &Value,
) -> Result<reqwest::Response> {
    let header_map = build_header_map(key, headers);
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
async fn connect_with_retry(
    client: &reqwest::Client,
    url: &str,
    key: &str,
    headers: &[(String, String)],
    body: &Value,
    tx: &mpsc::Sender<LlmEvent>,
) -> Result<reqwest::Response> {
    let mut attempt: u8 = 0;
    loop {
        attempt = attempt.saturating_add(1);
        match send_request(client, url, key, headers, body).await {
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
    let input_tokens = u
        .get("prompt_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or_default();
    let output_tokens = u
        .get("completion_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or_default();
    let total_tokens = u
        .get("total_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or_default();

    // Prompt-caching accounting. Provider naming is inconsistent, so accept
    // every known variant and normalize into two fields (see `Usage` docs):
    //   cache_read:     cache_read_input_tokens | cache_read
    //                   | prompt_tokens_details.cached_tokens (OpenAI native)
    //   cache_creation: cache_creation_input_tokens | cache_write
    let cache_read_tokens = first_u64(u, &["cache_read_input_tokens", "cache_read"])
        .or_else(|| {
            u.get("prompt_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
                .and_then(|v| v.as_u64())
        })
        .unwrap_or_default();
    let cache_creation_tokens =
        first_u64(u, &["cache_creation_input_tokens", "cache_write"]).unwrap_or_default();

    Usage {
        input_tokens,
        output_tokens,
        total_tokens,
        cache_read_tokens,
        cache_creation_tokens,
    }
}

/// Return the first `u64` found under any of `keys` in `obj`, or `None`.
/// Used by `parse_usage` to collapse provider-specific cache-field aliases
/// (checked in priority order) into one normalized value.
fn first_u64(obj: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|k| obj.get(*k).and_then(|v| v.as_u64()))
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(n).collect();
        format!("{truncated}...")
    }
}

/// Build the HTTP header map for a chat request. Built-in headers
/// (`authorization`, `content-type`, `accept`) are applied first; entries in
/// `custom` then override any built-in with the same (case-insensitive) name.
/// Malformed custom entries (invalid header name or value bytes) are silently
/// skipped so one bad entry can't break the whole stream. Pure and
/// side-effect-free so the override/merge behavior is unit-testable.
pub fn build_header_map(key: &str, custom: &[(String, String)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&format!("Bearer {key}")) {
        map.insert(AUTHORIZATION, v);
    }
    map.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    map.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
    for (name, value) in custom {
        if let (Ok(n), Ok(v)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            map.insert(n, v);
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard: the default read timeout must stay at 600 s (10 min).
    #[test]
    fn default_read_timeout_is_600s() {
        assert_eq!(DEFAULT_READ_TIMEOUT, Duration::from_secs(600));
    }

    // ---- parse_usage: base fields + cache-token normalization ----

    fn usage_json(s: &str) -> Value {
        serde_json::from_str(s).expect("valid usage json")
    }

    #[test]
    fn parse_usage_reads_openai_base_fields() {
        let u = parse_usage(&usage_json(
            r#"{"prompt_tokens":100,"completion_tokens":40,"total_tokens":140}"#,
        ));
        assert_eq!(u.input_tokens, 100);
        assert_eq!(u.output_tokens, 40);
        assert_eq!(u.total_tokens, 140);
        assert_eq!(u.cache_read_tokens, 0);
        assert_eq!(u.cache_creation_tokens, 0);
    }

    #[test]
    fn parse_usage_reads_anthropic_cache_fields() {
        let u = parse_usage(&usage_json(
            r#"{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15,
               "cache_read_input_tokens":42,"cache_creation_input_tokens":7}"#,
        ));
        assert_eq!(u.cache_read_tokens, 42);
        assert_eq!(u.cache_creation_tokens, 7);
    }

    #[test]
    fn parse_usage_reads_short_aliases() {
        let u = parse_usage(&usage_json(
            r#"{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2,
               "cache_read":8,"cache_write":3}"#,
        ));
        assert_eq!(u.cache_read_tokens, 8);
        assert_eq!(u.cache_creation_tokens, 3);
    }

    #[test]
    fn parse_usage_reads_openai_cached_tokens() {
        let u = parse_usage(&usage_json(
            r#"{"prompt_tokens":300,"completion_tokens":20,"total_tokens":320,
               "prompt_tokens_details":{"cached_tokens":9000}}"#,
        ));
        assert_eq!(u.cache_read_tokens, 9000);
        assert_eq!(u.cache_creation_tokens, 0);
    }

    #[test]
    fn parse_usage_prefers_explicit_anthropic_key_over_alias() {
        let u = parse_usage(&usage_json(
            r#"{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2,
               "cache_read_input_tokens":7,"cache_read":9}"#,
        ));
        assert_eq!(u.cache_read_tokens, 7);
    }

    #[test]
    fn parse_usage_missing_cache_fields_default_to_zero() {
        let u = parse_usage(&usage_json(
            r#"{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0}"#,
        ));
        assert_eq!(u.cache_read_tokens, 0);
        assert_eq!(u.cache_creation_tokens, 0);
    }

    #[test]
    fn parse_usage_empty_object_is_all_zeros() {
        let u = parse_usage(&usage_json("{}"));
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.output_tokens, 0);
        assert_eq!(u.total_tokens, 0);
        assert_eq!(u.cache_read_tokens, 0);
        assert_eq!(u.cache_creation_tokens, 0);
    }

    #[test]
    fn first_u64_returns_first_present_key() {
        let obj = usage_json(r#"{"a":10,"b":20}"#);
        assert_eq!(first_u64(&obj, &["a", "b"]), Some(10));
        assert_eq!(first_u64(&obj, &["b", "a"]), Some(20));
        assert_eq!(first_u64(&obj, &["missing"]), None);
    }
}

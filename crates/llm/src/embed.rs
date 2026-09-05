//! OpenAI-compatible `/embeddings` support.
//!
//! Everything that can be a pure function is one: request-body construction
//! and response parsing are directly unit-testable with no HTTP involved.
//! The async POST (`post_embeddings`) and the sync bridge (`embeddings_via`)
//! adapt those pure pieces to the `ChatStream::embed` contract.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::header::{HeaderValue, ACCEPT};
use tracing::warn;

use crate::client::{build_header_map, ChatClient};
use crate::http_date::parse_http_date_to_secs;
use crate::retry::{
    backoff_delay, backoff_duration, retry_decision, retry_delay, AttemptOutcome, RetryDecision,
};

/// Total attempts for one `/embeddings` call (1 initial + 2 retries). Kept
/// separate from the streaming budgets in `retry.rs`: embeddings is a short,
/// idempotent JSON round-trip, so a small bounded budget absorbs upstream
/// blips (429/5xx/transport) without amplifying load.
pub(crate) const EMBED_MAX_ATTEMPTS: u8 = 3;

/// Per-request total timeout for `/embeddings`, much tighter than the shared
/// client's streaming `read_timeout`: a small JSON request either answers
/// quickly or the attempt counts as a transport failure and feeds the retry
/// policy instead of pinning the call for minutes.
const EMBED_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Build the JSON body for a `/embeddings` POST:
/// `{"model": <model>, "input": [<text>, ...]}`. Pure; input order is kept.
pub fn build_embed_body(texts: &[String], model: &str) -> String {
    let body = serde_json::json!({ "model": model, "input": texts });
    serde_json::to_string(&body).expect("serializing strings into JSON cannot fail")
}

/// Parse an `/embeddings` response body into one vector per input text, in
/// input order. OpenAI-compatible servers return a `data` array whose entries
/// carry an `index` matching the input position; entries are therefore sorted
/// by `index` when every entry has one (stable, so equal indices keep arrival
/// order), and otherwise left in arrival order. Pure: no allocation beyond
/// the output, no I/O, no partial failure after decoding.
pub fn parse_embeddings_response(body: &[u8]) -> Result<Vec<Vec<f32>>> {
    let parsed: serde_json::Value =
        serde_json::from_slice(body).context("decode embeddings response body")?;
    let data = parsed
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| anyhow!("embeddings response is missing the `data` array"))?;
    if data.is_empty() {
        return Err(anyhow!("embeddings response `data` array is empty"));
    }
    let mut entries: Vec<(Option<u64>, Vec<f32>)> = Vec::with_capacity(data.len());
    for item in data {
        let raw = item
            .get("embedding")
            .and_then(|e| e.as_array())
            .ok_or_else(|| anyhow!("embeddings entry is missing the `embedding` array"))?;
        let mut vec = Vec::with_capacity(raw.len());
        for component in raw {
            let n = component
                .as_f64()
                .ok_or_else(|| anyhow!("embedding component is not a number"))?;
            vec.push(n as f32);
        }
        let index = item.get("index").and_then(|v| v.as_u64());
        entries.push((index, vec));
    }
    if entries.iter().all(|(i, _)| i.is_some()) {
        entries.sort_by_key(|(i, _)| i.unwrap_or(0));
    }
    Ok(entries.into_iter().map(|(_, v)| v).collect())
}

/// POST `{base_url}/embeddings` and parse the vectors. Transient failures —
/// whitelisted statuses (408/425/429/5xx) and transport errors with no status
/// — are retried up to [`EMBED_MAX_ATTEMPTS`] times, delegating every
/// retry-vs-fail-vs-done decision to the pure `retry_decision` policy.
/// On a retryable status the server's `Retry-After` hint (integer seconds or
/// HTTP-date, parsed exactly like the chat path) is honored — floored at 1 s,
/// capped at [`crate::retry::RETRY_AFTER_MAX_SECS`], and combined with the
/// local backoff by the shared pure `retry_delay` — in a single sleep.
/// Non-retryable statuses fail immediately. Response parsing and the
/// vector/text count check are never retried: a malformed or misaligned body
/// is deterministic server behavior, not a transient blip.
async fn post_embeddings(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    custom_headers: &[(String, String)],
    texts: &[String],
    model: &str,
) -> Result<Vec<Vec<f32>>> {
    let url = format!("{}/embeddings", base_url.trim_end_matches('/'));
    // Same header treatment as chat (auth + custom overrides), but `/embeddings`
    // is a plain JSON round-trip, so accept JSON instead of the SSE default.
    let mut headers = build_header_map(api_key, custom_headers)?;
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    // `reqwest` consumes the body per attempt; a `String` clones cheaply, so
    // every retry re-sends a byte-identical request.
    let body = build_embed_body(texts, model);

    let mut attempt: u8 = 0;
    loop {
        attempt = attempt.saturating_add(1);
        let send_result = client
            .post(&url)
            .headers(headers.clone())
            .body(body.clone())
            // Total per-attempt budget (connect + headers + body), so a
            // stalled upstream cannot pin the call for the streaming timeout.
            .timeout(EMBED_REQUEST_TIMEOUT)
            .send()
            .await;

        let resp = match send_result {
            Ok(resp) => resp,
            // Network/transport error (no HTTP status) — always transient.
            Err(e) => {
                if retry_decision(AttemptOutcome::RetryableError, attempt, EMBED_MAX_ATTEMPTS)
                    == RetryDecision::Fail
                {
                    return Err(anyhow::Error::from(e).context(format!(
                        "send embeddings request to {url} failed after {attempt} attempts"
                    )));
                }
                warn!(
                    attempt,
                    max = EMBED_MAX_ATTEMPTS,
                    error = %e,
                    "embeddings send error, will retry"
                );
                backoff_delay(attempt).await;
                continue;
            }
        };

        let status = resp.status();
        if retry_decision(
            AttemptOutcome::from_status(status),
            attempt,
            EMBED_MAX_ATTEMPTS,
        ) == RetryDecision::Retry
        {
            // Capture `Retry-After` BEFORE draining the body (headers stay
            // readable after `text()`, but reading first keeps the intent
            // explicit), then drain so the connection is not left half-read.
            // Parsing mirrors the chat path exactly: integer seconds first,
            // HTTP-date as fallback.
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok().or_else(|| parse_http_date_to_secs(s)));
            let _ = resp.text().await;
            warn!(
                attempt,
                max = EMBED_MAX_ATTEMPTS,
                status = %status,
                retry_after_secs = ?retry_after,
                "embeddings upstream error, will retry"
            );
            // One computed sleep: the bounded server hint (floored/capped by
            // `retry_delay`) or the bare jittered backoff, whichever is
            // longer — never both.
            let delay = retry_delay(retry_after, backoff_duration(attempt));
            tokio::time::sleep(delay).await;
            continue;
        }

        let bytes = resp
            .bytes()
            .await
            .context("read embeddings response body")?;
        if !status.is_success() {
            return Err(anyhow!(
                "embeddings request failed: upstream {status} after {attempt} attempts; body: {}",
                truncate_body(&bytes, 512)
            ));
        }
        let vectors = parse_embeddings_response(&bytes)?;
        // Contract: exactly one vector per input text. A server that returns a
        // different count cannot be aligned with the inputs, so fail loudly
        // instead of silently returning misaligned vectors.
        if vectors.len() != texts.len() {
            return Err(anyhow!(
                "embeddings response returned {} vectors for {} input texts",
                vectors.len(),
                texts.len()
            ));
        }
        return Ok(vectors);
    }
}

/// Lossily decode and char-truncate a response body for error messages.
fn truncate_body(bytes: &[u8], max_chars: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    if text.chars().count() <= max_chars {
        text.into_owned()
    } else {
        let cut: String = text.chars().take(max_chars).collect();
        format!("{cut}...")
    }
}

/// Blocking entry point behind the sync `ChatStream::embed` trait method.
///
/// `ChatStream::embed` is not async, so the POST must be driven to completion
/// from whatever context called it:
///
/// * On a **multi-thread** runtime (the production session runtime) the future
///   runs via `block_in_place` on the caller's own runtime, so the shared
///   `reqwest::Client` connection pool and timers stay on the same driver.
/// * Otherwise (a current-thread runtime — the default `#[tokio::test]`
///   flavor — or a fully synchronous caller) it runs on a short-lived thread
///   with its own single-threaded runtime, which can neither deadlock the
///   caller's runtime nor stall one of its workers.
pub(crate) fn embeddings_via(
    client: &ChatClient,
    texts: &[String],
    model: &str,
) -> Result<Vec<Vec<f32>>> {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| {
                handle.block_on(post_embeddings(
                    &client.http,
                    &client.base_url,
                    &client.api_key,
                    &client.headers,
                    texts,
                    model,
                ))
            })
        }
        _ => {
            // The worker thread needs owned data (the closure must be
            // 'static); cloning here is cheap next to the HTTP round-trip.
            let (http, base_url, api_key, headers, texts, model) = (
                client.http.clone(),
                client.base_url.clone(),
                client.api_key.clone(),
                client.headers.clone(),
                texts.to_vec(),
                model.to_string(),
            );
            let worker = std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("build embeddings runtime")?;
                rt.block_on(post_embeddings(
                    &http, &base_url, &api_key, &headers, &texts, &model,
                ))
            });
            worker
                .join()
                .map_err(|_| anyhow!("embeddings worker thread panicked"))?
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChatStream;

    #[test]
    fn build_embed_body_encodes_model_and_inputs_in_order() {
        let texts = vec!["first".to_string(), "sécond".to_string()];
        let body = build_embed_body(&texts, "text-embedding-3-small");
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["model"], "text-embedding-3-small");
        assert_eq!(parsed["input"][0], "first");
        assert_eq!(parsed["input"][1], "sécond");
        assert_eq!(parsed["input"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn parse_reads_vectors_in_index_order() {
        // Deliberately shuffled `data` (index 1 before 0): parse must restore
        // input order via the `index` field.
        let body = br#"{"object":"list","data":[
            {"object":"embedding","index":1,"embedding":[0.5,0.5]},
            {"object":"embedding","index":0,"embedding":[0.1,0.9]}
        ],"model":"m","usage":{"prompt_tokens":3,"total_tokens":3}}"#;
        let out = parse_embeddings_response(body).unwrap();
        assert_eq!(out, vec![vec![0.1, 0.9], vec![0.5, 0.5]]);
    }

    #[test]
    fn parse_keeps_arrival_order_when_index_absent() {
        let body = br#"{"data":[
            {"embedding":[1.0]},
            {"embedding":[2.0]}
        ]}"#;
        let out = parse_embeddings_response(body).unwrap();
        assert_eq!(out, vec![vec![1.0], vec![2.0]]);
    }

    #[test]
    fn parse_converts_f64_components_to_f32() {
        let body = br#"{"data":[{"index":0,"embedding":[0.25,1.5,-2]}]}"#;
        let out = parse_embeddings_response(body).unwrap();
        assert_eq!(out, vec![vec![0.25f32, 1.5, -2.0]]);
    }

    #[test]
    fn parse_rejects_empty_data() {
        assert!(parse_embeddings_response(br#"{"data":[]}"#).is_err());
    }

    #[test]
    fn parse_rejects_missing_embedding_field() {
        assert!(parse_embeddings_response(br#"{"data":[{"index":0}]}"#).is_err());
    }

    #[test]
    fn parse_rejects_garbage_and_missing_data() {
        assert!(parse_embeddings_response(b"not json").is_err());
        assert!(parse_embeddings_response(br#"{"object":"list"}"#).is_err());
    }

    #[test]
    fn parse_rejects_non_numeric_component() {
        assert!(parse_embeddings_response(br#"{"data":[{"embedding":["x"]}]"#).is_err());
    }

    // The default trait implementation must bail cleanly (old implementors
    // keep compiling and fail loudly instead of silently doing nothing).
    struct NoEmbed;
    impl crate::ChatStream for NoEmbed {
        fn chat_stream(
            &self,
            _req: crate::ChatRequest,
        ) -> Result<tokio::sync::mpsc::Receiver<crate::LlmEvent>> {
            unreachable!("not exercised by this test")
        }
        fn backend(&self) -> &'static str {
            "no-embed"
        }
    }

    #[test]
    fn default_embed_bails_with_backend_name() {
        let err = NoEmbed.embed(&["x".into()][..], "m").unwrap_err();
        assert!(err.to_string().contains("no-embed"), "got: {err:#}");
    }

    #[test]
    fn truncate_body_caps_output_and_survives_invalid_utf8() {
        assert_eq!(truncate_body(b"hello", 8), "hello");
        assert_eq!(truncate_body(b"abcdefghij", 4), "abcd...");
        assert_eq!(truncate_body(&[0xff, 0xfe], 8), "\u{fffd}\u{fffd}");
    }
}

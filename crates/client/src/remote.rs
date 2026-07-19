//! HTTP/SSE client for an `opencoder server`. A `Remote` is a thin handle over
//! a `reqwest::Client` that mirrors the server API: health, list/create
//! sessions, fetch messages, snapshot event seq, post a prompt, switch
//! agent/model, interrupt, and stream `/events`. Every request carries the
//! bearer token via the `Authorization` header.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use opencoder_core::{Message, SseEvt};
use tokio::sync::mpsc;
use tracing::warn;

use crate::sse::SseFrameDecoder;

const READ_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone)]
pub struct Remote {
    base_url: String,
    token: String,
    http: reqwest::Client,
}

impl Remote {
    pub fn new(base_url: &str, token: &str) -> Result<Self> {
        // Proxy-aware client with loopback bypass: a configured/env proxy is
        // honored for remote hosts, but localhost (and the test server bound
        // to 127.0.0.1) always connects directly so proxy env vars don't
        // break local connections.
        let http = opencoder_core::net::build_http_client_with_read_timeout(None, READ_TIMEOUT)?;
        Ok(Remote {
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            http,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// GET /api/health → true if the server reports ok (and the token is valid).
    pub async fn health(&self) -> Result<bool> {
        let resp = self
            .http
            .get(self.url("/api/health"))
            .bearer_auth(&self.token)
            .send()
            .await
            .context("health request")?;
        if !resp.status().is_success() {
            return Ok(false);
        }
        let v: serde_json::Value = resp.json().await.context("health json")?;
        Ok(v.get("ok").and_then(|o| o.as_bool()).unwrap_or(false))
    }

    /// List sessions (most-recent first). Returns the raw list items so the
    /// caller can read `id`/`preview` without a fixed schema.
    pub async fn list_sessions(&self) -> Result<Vec<serde_json::Value>> {
        let resp = self
            .http
            .get(self.url("/api/sessions"))
            .bearer_auth(&self.token)
            .send()
            .await
            .context("list sessions")?;
        let resp = ensure_ok(resp, "list sessions").await?;
        let v: serde_json::Value = resp.json().await.context("list json")?;
        Ok(v.get("sessions")
            .cloned()
            .and_then(|s| serde_json::from_value(s).ok())
            .unwrap_or_default())
    }

    /// POST /api/sessions → the new session id.
    pub async fn create_session(
        &self,
        agent: Option<&str>,
        model: Option<&str>,
    ) -> Result<String> {
        let mut body = serde_json::json!({});
        if let Some(a) = agent {
            body["agent"] = serde_json::json!(a);
        }
        if let Some(m) = model {
            body["model"] = serde_json::json!(m);
        }
        let resp = self
            .http
            .post(self.url("/api/sessions"))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .context("create session")?;
        let resp = ensure_ok(resp, "create session").await?;
        let v: serde_json::Value = resp.json().await.context("create json")?;
        v.get("id")
            .and_then(|i| i.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("create session: missing id"))
    }

    /// GET /api/sessions/:id/messages → the message transcript.
    pub async fn get_messages(&self, id: &str) -> Result<Vec<Message>> {
        let resp = self
            .http
            .get(self.url(&format!("/api/sessions/{id}/messages")))
            .bearer_auth(&self.token)
            .send()
            .await
            .context("get messages")?;
        let resp = ensure_ok(resp, "get messages").await?;
        let v: serde_json::Value = resp.json().await.context("messages json")?;
        let msgs = v
            .get("messages")
            .cloned()
            .unwrap_or(serde_json::Value::Array(vec![]));
        serde_json::from_value::<Vec<Message>>(msgs).context("deserialize messages")
    }

    /// GET /api/sessions/:id/seq → the highest persisted event seq (0 if none).
    pub async fn last_event_seq(&self, id: &str) -> Result<i64> {
        let resp = self
            .http
            .get(self.url(&format!("/api/sessions/{id}/seq")))
            .bearer_auth(&self.token)
            .send()
            .await
            .context("get seq")?;
        let resp = ensure_ok(resp, "get seq").await?;
        let v: serde_json::Value = resp.json().await.context("seq json")?;
        Ok(v.get("seq").and_then(|s| s.as_i64()).unwrap_or(0))
    }

    /// POST /api/sessions/:id/prompt → the admitted input seq.
    #[allow(clippy::too_many_arguments)]
    pub async fn post_prompt(
        &self,
        id: &str,
        prompt: &str,
        delivery: Option<&str>,
        agent: Option<&str>,
        model: Option<&str>,
    ) -> Result<i64> {
        let mut body = serde_json::json!({ "prompt": prompt });
        if let Some(d) = delivery {
            body["delivery"] = serde_json::json!(d);
        }
        if let Some(a) = agent {
            body["agent"] = serde_json::json!(a);
        }
        if let Some(m) = model {
            body["model"] = serde_json::json!(m);
        }
        let resp = self
            .http
            .post(self.url(&format!("/api/sessions/{id}/prompt")))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .context("post prompt")?;
        let resp = ensure_ok(resp, "post prompt").await?;
        let v: serde_json::Value = resp.json().await.context("prompt json")?;
        // server returns { "admitted_seq": N, "ok": true }; fall back to "seq".
        Ok(v
            .get("admitted_seq")
            .or_else(|| v.get("seq"))
            .and_then(|s| s.as_i64())
            .unwrap_or(0))
    }

    pub async fn switch_agent(&self, id: &str, value: &str) -> Result<()> {
        self.switch(id, "agent", value).await
    }

    pub async fn switch_model(&self, id: &str, value: &str) -> Result<()> {
        self.switch(id, "model", value).await
    }

    async fn switch(&self, id: &str, kind: &str, value: &str) -> Result<()> {
        let body = serde_json::json!({ "value": value });
        let resp = self
            .http
            .post(self.url(&format!("/api/sessions/{id}/{kind}")))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .context("switch")?;
        let _ = ensure_ok(resp, "switch").await?;
        Ok(())
    }

    pub async fn interrupt(&self, id: &str) -> Result<()> {
        let resp = self
            .http
            .post(self.url(&format!("/api/sessions/{id}/interrupt")))
            .bearer_auth(&self.token)
            .send()
            .await
            .context("interrupt")?;
        let _ = ensure_ok(resp, "interrupt").await?;
        Ok(())
    }

    /// Subscribe to a session's SSE `/events` stream from `after`. Returns a
    /// channel of decoded `SseEvt`; the channel closes when the server ends the
    /// stream. The caller normally breaks on a `done`/`error` event rather than
    /// the channel close (the server keeps the stream open with keep-alive).
    pub fn events(&self, id: &str, after: i64) -> Result<mpsc::Receiver<SseEvt>> {
        let (tx, rx) = mpsc::channel::<SseEvt>(128);
        let url = self.url(&format!("/api/sessions/{id}/events?after={after}"));
        let token = self.token.clone();
        let http = self.http.clone();
        tokio::spawn(async move {
            if let Err(e) = run_stream(http, url, token, tx.clone()).await {
                let _ = tx
                    .send(SseEvt {
                        kind: "error".into(),
                        data: serde_json::json!({ "error": format!("stream failed: {e:#}") }),
                        ts: opencoder_core::message::now_ms(),
                        seq: None,
                    })
                    .await;
            }
        });
        Ok(rx)
    }
}

async fn run_stream(
    http: reqwest::Client,
    url: String,
    token: String,
    tx: mpsc::Sender<SseEvt>,
) -> Result<()> {
    let resp = http
        .get(&url)
        .bearer_auth(&token)
        .header("accept", "text/event-stream")
        .send()
        .await
        .context("events request")?;
    if !resp.status().is_success() {
        return Err(anyhow!(
            "events: HTTP {} {}",
            resp.status().as_u16(),
            resp.status().canonical_reason().unwrap_or("")
        ));
    }
    let mut stream = resp.bytes_stream();
    let mut dec = SseFrameDecoder::new();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.context("read events chunk")?;
        dec.push(&bytes);
        for frame in dec.drain() {
            let kind = frame.event.unwrap_or_else(|| "message".into());
            let data: serde_json::Value =
                serde_json::from_str(&frame.data).unwrap_or(serde_json::Value::Null);
            let evt = SseEvt {
                kind,
                data,
                ts: opencoder_core::message::now_ms(),
                seq: None,
            };
            if tx.send(evt).await.is_err() {
                // subscriber dropped (client exited) — stop draining
                return Ok(());
            }
        }
    }
    for frame in dec.flush_remaining() {
        let kind = frame.event.unwrap_or_else(|| "message".into());
        let data: serde_json::Value =
            serde_json::from_str(&frame.data).unwrap_or(serde_json::Value::Null);
        let _ = tx
            .send(SseEvt {
                kind,
                data,
                ts: opencoder_core::message::now_ms(),
                seq: None,
            })
            .await;
    }
    Ok(())
}

/// Turn a non-success HTTP response into an error carrying the server's
/// message body when available (e.g. the JSON `{ "error": ... }`). On success
/// the (unchanged) response is returned so the caller can still read its body.
async fn ensure_ok(resp: reqwest::Response, what: &str) -> Result<reqwest::Response> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    warn!(%status, what, body = %body, "server rejected request");
    Err(anyhow!("{what}: HTTP {status}: {body}"))
}

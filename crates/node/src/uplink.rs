//! REST uplink from an execution node to its central server.
//!
//! One thin handle over a proxy-aware `reqwest::Client` (loopback always
//! bypasses proxies). Every request is HMAC-signed with the shared token via
//! [`opencoder_core::auth_sig`]; every non-2xx response becomes an `anyhow`
//! error that embeds the server's body.

use std::time::Duration;

use anyhow::{Context, Result};
use opencoder_core::auth_sig;
use opencoder_core::net::build_http_client_with_read_timeout;
use opencoder_core::node_protocol::{
    ClaimResponse, FetchMessagesResult, NodeEventBatch, NodeHeartbeatResponse, NodeRegisterRequest,
    NodeRegisterResponse, NodeStatusReport,
};
use tracing::warn;

/// Per-read idle timeout for control-plane calls. Streams never pass through
/// here, so a moderately tight bound keeps a wedged server from stalling the
/// main loop forever.
const READ_TIMEOUT: Duration = Duration::from_secs(120);

/// Independent, deliberately SHORT budget for one heartbeat round trip
/// (connect + send + response headers + body). [`READ_TIMEOUT`] (120s) is
/// far wider than the server's liveness window (`STALE_AFTER_MS = 20s`, see
/// `crates/web/src/nodes_state.rs`), so a single wedged beat used to make a
/// live node look silent long enough for `converge_lost_node_tasks` to fold
/// its running tasks into `error("node lost")` — fake failures plus burned
/// tokens.
///
/// Liveness budget arithmetic (worst silent gap between two served beats):
/// heartbeat timeout (5s) + tick interval (default 5s) ≈ 10s < 20s, about 2×
/// headroom. After one timeout the next tick fires immediately
/// (`MissedTickBehavior::Skip` collapses the beats that elapsed in flight),
/// so timeouts never stack; a beat that fails FAST (weak network) costs no
/// liveness at all — the loop just waits for the next tick, which is the
/// built-in single retry. See also `runner::DEFAULT_HEARTBEAT_INTERVAL`.
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);

/// Worker-side REST client handle. Cheap to clone (`reqwest::Client` is an
/// internal Arc), so per-task background duties can own a copy.
#[derive(Clone)]
pub struct Uplink {
    http: reqwest::Client,
    base: String,
    token: String,
    /// Per-heartbeat round-trip budget; defaults to [`HEARTBEAT_TIMEOUT`],
    /// injectable for tests via [`Uplink::with_heartbeat_timeout`].
    heartbeat_timeout: Duration,
}

impl Uplink {
    /// Build an uplink against `base` (trailing slashes trimmed) with the
    /// resolved bearer token. Transport construction errors are fatal here.
    pub fn new(base: &str, token: &str) -> Result<Self> {
        Uplink::with_heartbeat_timeout(base, token, HEARTBEAT_TIMEOUT)
    }

    /// Like [`Uplink::new`], but overrides the per-heartbeat round-trip
    /// budget ([`HEARTBEAT_TIMEOUT`] by default). This is the injection seam
    /// for tests: shrinking it to milliseconds proves timeout-then-recovery
    /// deterministically instead of waiting on real network stalls.
    /// Production callers always use [`Uplink::new`].
    pub fn with_heartbeat_timeout(base: &str, token: &str, d: Duration) -> Result<Self> {
        let http = build_http_client_with_read_timeout(None, READ_TIMEOUT)?;
        Ok(Uplink {
            http,
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            heartbeat_timeout: d,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// POST /api/nodes/register — announce (or re-announce) this node.
    pub async fn register(
        &self,
        name: &str,
        version: &str,
        workdir: Option<&str>,
    ) -> Result<NodeRegisterResponse> {
        let body = NodeRegisterRequest {
            name: name.to_string(),
            version: Some(version.to_string()),
            workdir: workdir.map(str::to_string),
            addr: None,
        };
        let resp = self
            .signed_request(reqwest::Method::POST, "/api/nodes/register", Some(&body))
            .await
            .context("register node")?;
        let resp = ensure_ok(resp, "register node").await?;
        resp.json().await.context("register node json")
    }

    /// POST /api/nodes/:id/heartbeat — liveness touch + cancel-command poll.
    ///
    /// The WHOLE round trip is bounded by [`HEARTBEAT_TIMEOUT`] (or the
    /// test-injected override): a server that accepts the request but never
    /// answers — or stalls the body — degrades to a plain `Err` instead of
    /// keeping the beat in flight past the server's liveness window.
    /// Callers only `warn!` on `Err` and wait for the next tick.
    pub async fn heartbeat(&self, node_id: &str) -> Result<NodeHeartbeatResponse> {
        let round_trip = async {
            let resp = self
                .signed_request(
                    reqwest::Method::POST,
                    &format!("/api/nodes/{node_id}/heartbeat"),
                    Some(&serde_json::json!({})),
                )
                .await
                .context("heartbeat")?;
            let resp = ensure_ok(resp, "heartbeat").await?;
            resp.json().await.context("heartbeat json")
        };
        match tokio::time::timeout(self.heartbeat_timeout, round_trip).await {
            Ok(res) => res,
            Err(_) => Err(anyhow::anyhow!(
                "heartbeat timed out after {:?} (HEARTBEAT_TIMEOUT budget)",
                self.heartbeat_timeout
            )),
        }
    }

    /// GET /api/nodes/tasks/claim?node_id= — FIFO single-active claim.
    ///
    /// The reply is the [`ClaimResponse`] envelope: a durable task and/or a
    /// control task (P3 message relay). `204 No Content` maps to an empty
    /// envelope (both fields `None`), so callers need no special case.
    pub async fn claim_next(&self, node_id: &str) -> Result<ClaimResponse> {
        let pq = format!(
            "/api/nodes/tasks/claim?node_id={}",
            urlencode_component(node_id)
        );
        let resp = self
            .signed_request(reqwest::Method::GET, &pq, None::<&serde_json::Value>)
            .await
            .context("claim task")?;
        if resp.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(ClaimResponse::default());
        }
        let resp = ensure_ok(resp, "claim task").await?;
        resp.json().await.context("claim task json")
    }

    /// POST /api/nodes/tasks/:tid/events — upload one ordered event batch.
    pub async fn upload_events(&self, task_id: &str, batch: NodeEventBatch) -> Result<()> {
        let resp = self
            .signed_request(
                reqwest::Method::POST,
                &format!("/api/nodes/tasks/{task_id}/events"),
                Some(&batch),
            )
            .await
            .context("upload events")?;
        let _ = ensure_ok(resp, "upload events").await?;
        Ok(())
    }

    /// POST /api/nodes/tasks/:tid/status — terminal transition report.
    pub async fn report_status(
        &self,
        task_id: &str,
        status: &str,
        error: Option<String>,
    ) -> Result<()> {
        let report = NodeStatusReport {
            status: status.to_string(),
            error,
        };
        let resp = self
            .signed_request(
                reqwest::Method::POST,
                &format!("/api/nodes/tasks/{task_id}/status"),
                Some(&report),
            )
            .await
            .context("report status")?;
        let _ = ensure_ok(resp, "report status").await?;
        Ok(())
    }
}

impl Uplink {
    /// Single signed egress path: serialize the JSON body (if any), compute
    /// the HMAC over `{METHOD}\n{path_and_query}\n{ts}\n{sha256(body)}`, and
    /// attach the signature headers. Every control-plane call goes through
    /// here, so the wire format can never drift per-call-site.
    /// POST /api/nodes/:id/control_result — upload one control result
    /// (P3 message relay). The server answers `{"resolved": bool}` and ALWAYS
    /// 200: an unknown/stale control id is a no-op, never a retryable error.
    pub async fn post_control_result(
        &self,
        node_id: &str,
        result: &FetchMessagesResult,
    ) -> Result<()> {
        let resp = self
            .signed_request(
                reqwest::Method::POST,
                &format!("/api/nodes/{node_id}/control_result"),
                Some(result),
            )
            .await
            .context("upload control result")?;
        let _ = ensure_ok(resp, "upload control result").await?;
        Ok(())
    }

    async fn signed_request<T: serde::Serialize>(
        &self,
        method: reqwest::Method,
        path_and_query: &str,
        body: Option<&T>,
    ) -> Result<reqwest::Response> {
        let body_bytes = match body {
            Some(b) => serde_json::to_vec(b).context("serialize request body")?,
            None => Vec::new(),
        };
        let ts = opencoder_core::message::now_ms();
        let canon = auth_sig::canonical(method.as_str(), path_and_query, ts, &body_bytes);
        let sig = auth_sig::sign_hex(&self.token, &canon);
        self.http
            .request(method, self.url(path_and_query))
            .header(auth_sig::TS_HEADER, ts.to_string())
            .header(auth_sig::SIG_HEADER, sig)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body_bytes)
            .send()
            .await
            .context("send signed request")
    }
}

/// Percent-encode one query component (ids are ULIDs; this is belt-and-braces).
fn urlencode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Accept only 2xx; embed the server's body in the error so operators see the
/// rejection reason without re-running the request.
async fn ensure_ok(resp: reqwest::Response, what: &'static str) -> Result<reqwest::Response> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    warn!(%status, what, body = %body, "server rejected uplink request");
    Err(anyhow::anyhow!("{what}: HTTP {status}: {body}"))
}

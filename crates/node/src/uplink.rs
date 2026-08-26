//! REST uplink from an execution node to its central server.
//!
//! One thin handle over a proxy-aware `reqwest::Client` (loopback always
//! bypasses proxies, mirroring `opencode client`'s transport). Every request
//! carries the bearer token; every non-2xx response becomes an `anyhow` error
//! that embeds the server's body — the same house style as
//! `crates/client/src/remote_ops.rs`, minus the SSE surface.

use std::time::Duration;

use anyhow::{Context, Result};
use opencoder_core::net::build_http_client_with_read_timeout;
use opencoder_core::node_protocol::{
    ClaimedTask, NodeEventBatch, NodeHeartbeatResponse, NodeRegisterRequest, NodeRegisterResponse,
    NodeStatusReport,
};
use tracing::warn;

/// Per-read idle timeout for control-plane calls. Streams never pass through
/// here, so a moderately tight bound keeps a wedged server from stalling the
/// main loop forever.
const READ_TIMEOUT: Duration = Duration::from_secs(120);

/// Worker-side REST client handle. Cheap to clone (`reqwest::Client` is an
/// internal Arc), so per-task background duties can own a copy.
#[derive(Clone)]
pub struct Uplink {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Uplink {
    /// Build an uplink against `base` (trailing slashes trimmed) with the
    /// resolved bearer token. Transport construction errors are fatal here.
    pub fn new(base: &str, token: &str) -> Result<Self> {
        let http = build_http_client_with_read_timeout(None, READ_TIMEOUT)?;
        Ok(Uplink {
            http,
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
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
        };
        let resp = self
            .http
            .post(self.url("/api/nodes/register"))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .context("register node")?;
        let resp = ensure_ok(resp, "register node").await?;
        resp.json().await.context("register node json")
    }

    /// POST /api/nodes/:id/heartbeat — liveness touch + cancel-command poll.
    pub async fn heartbeat(&self, node_id: &str) -> Result<NodeHeartbeatResponse> {
        let resp = self
            .http
            .post(self.url(&format!("/api/nodes/{node_id}/heartbeat")))
            .bearer_auth(&self.token)
            .json(&serde_json::json!({}))
            .send()
            .await
            .context("heartbeat")?;
        let resp = ensure_ok(resp, "heartbeat").await?;
        resp.json().await.context("heartbeat json")
    }

    /// GET /api/nodes/tasks/claim?node_id= — FIFO single-active claim.
    /// `204 No Content` means nothing is due and maps to `None`.
    pub async fn claim_next(&self, node_id: &str) -> Result<Option<ClaimedTask>> {
        let resp = self
            .http
            .get(self.url("/api/nodes/tasks/claim"))
            .query(&[("node_id", node_id)])
            .bearer_auth(&self.token)
            .send()
            .await
            .context("claim task")?;
        if resp.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(None);
        }
        let resp = ensure_ok(resp, "claim task").await?;
        Ok(Some(resp.json().await.context("claim task json")?))
    }

    /// POST /api/nodes/tasks/:tid/events — upload one ordered event batch.
    pub async fn upload_events(&self, task_id: &str, batch: NodeEventBatch) -> Result<()> {
        let resp = self
            .http
            .post(self.url(&format!("/api/nodes/tasks/{task_id}/events")))
            .bearer_auth(&self.token)
            .json(&batch)
            .send()
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
            .http
            .post(self.url(&format!("/api/nodes/tasks/{task_id}/status")))
            .bearer_auth(&self.token)
            .json(&report)
            .send()
            .await
            .context("report status")?;
        let _ = ensure_ok(resp, "report status").await?;
        Ok(())
    }
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

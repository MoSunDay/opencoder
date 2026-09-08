//! Submit → poll orchestration over a [`VikingTransport`].
//!
//! A dependency analysis runs for minutes on the hosted workflow. The tool
//! call submits one task, then polls the readback endpoint until the task
//! reaches a terminal state, the call is cancelled, or the budget elapses.
//! The VTA task survives a client disconnect — a cancelled poll can be
//! resumed later with a `status` action against the same task id.

use super::cli::VikingTransport;
use super::model::{classify_status, AnalysisItem, StatusPhase};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const TASKS_SUFFIX: &str = "/dependency-analysis/tasks";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(30);
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Current-source authority request-id prefix the VTA scheduler-off intake
/// accepts (the server expands `{link_id,record_id}` into the full item).
const V3_REQUEST_PREFIX: &str = "jianyingqa-current-v3-";

/// Build a unique current-source request id (`<prefix>` + 40 lowercase hex).
fn v3_request_id() -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(ulid::Ulid::new().to_bytes());
    hasher.update(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos().to_string())
            .unwrap_or_default(),
    );
    let hex = format!("{:x}", hasher.finalize());
    format!("{V3_REQUEST_PREFIX}{}", &hex[..40])
}

/// Submit one explicit five-tuple item (open-intake path) and return the
/// created child task id.
pub async fn submit(
    transport: &dyn VikingTransport,
    item: &AnalysisItem,
    workflow_override: Option<&str>,
) -> Result<String> {
    let body = super::model::build_create_body(item, workflow_override);
    let body_str = serde_json::to_string(&body)?;
    let resp = transport
        .request("POST", TASKS_SUFFIX, &[], &[], Some(&body_str))
        .await?;
    super::model::parse_created_task_id(&resp).map_err(|e| anyhow!(e))
}

/// Submit under the scheduler-off `current_source_only` intake: the body is
/// identity-only (`{link_id, record_id}` — a JianYingQA source-sync anchor)
/// and the request carries a `jianyingqa-current-v3-` request id header; the
/// server resolves the full five-tuple and source snapshot.
pub async fn submit_identity(
    transport: &dyn VikingTransport,
    link_id: u64,
    record_id: u64,
) -> Result<String> {
    let body = json!({ "items": [ { "link_id": link_id, "record_id": record_id } ] });
    let body_str = serde_json::to_string(&body)?;
    let request_id = v3_request_id();
    let headers = [("x-request-id".to_string(), request_id)];
    let resp = transport
        .request("POST", TASKS_SUFFIX, &headers, &[], Some(&body_str))
        .await?;
    super::model::parse_created_task_id(&resp).map_err(|e| anyhow!(e))
}

/// Read one task's current state.
pub async fn readback(transport: &dyn VikingTransport, task_id: &str) -> Result<Value> {
    let suffix = format!("{TASKS_SUFFIX}/{task_id}");
    transport.request("GET", &suffix, &[], &[], None).await
}

/// Outcome of waiting on a task.
#[derive(Debug)]
pub enum Terminal {
    /// Successful terminal state; the validated result payload.
    Done(Value),
    /// Failed/cancelled terminal state; the raw task (for error rendering).
    Failed(Value),
}

/// Poll until terminal, cancelled, or timed out. Transient transport errors
/// are logged and retried (bounded by the overall timeout) so a blip never
/// abandons a task that is still running server-side.
pub async fn poll_to_terminal(
    transport: &dyn VikingTransport,
    task_id: &str,
    cancel: &CancellationToken,
    poll_interval: Option<Duration>,
    timeout: Option<Duration>,
) -> Result<Terminal> {
    let interval = poll_interval.unwrap_or(DEFAULT_POLL_INTERVAL);
    let deadline = tokio::time::Instant::now() + timeout.unwrap_or(DEFAULT_TIMEOUT);
    loop {
        if cancel.is_cancelled() {
            bail!("dependency analysis cancelled; resume later with task_id={task_id}");
        }
        match readback(transport, task_id).await {
            Ok(task) => {
                match classify_status(task.get("status").and_then(Value::as_str).unwrap_or("")) {
                    StatusPhase::Success => return Ok(Terminal::Done(task)),
                    StatusPhase::Failed => return Ok(Terminal::Failed(task)),
                    StatusPhase::Active => {}
                }
            }
            Err(error) => {
                tracing::warn!(task_id, %error, "dependency readback failed; retrying");
            }
        }
        if tokio::time::Instant::now() >= deadline {
            bail!(
                "dependency analysis {task_id} not terminal within budget; resume later with task_id={task_id}"
            );
        }
        tokio::select! {
            _ = cancel.cancelled() => {
                bail!("dependency analysis cancelled; resume later with task_id={task_id}");
            }
            _ = tokio::time::sleep(interval) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// One recorded call: (method, suffix, headers, body).
    type RecordedCall = (String, String, Vec<(String, String)>, Option<String>);

    /// Scripted transport: pops the next canned response for each request,
    /// recording the request it saw.
    #[derive(Default)]
    struct MockTransport {
        responses: Mutex<Vec<Value>>,
        calls: Mutex<Vec<RecordedCall>>,
    }

    impl MockTransport {
        fn new(responses: Vec<Value>) -> Self {
            Self {
                responses: Mutex::new(responses),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl VikingTransport for MockTransport {
        async fn request(
            &self,
            method: &str,
            suffix: &str,
            headers: &[(String, String)],
            _query: &[(String, String)],
            body: Option<&str>,
        ) -> Result<Value> {
            self.calls.lock().unwrap().push((
                method.to_string(),
                suffix.to_string(),
                headers.to_vec(),
                body.map(str::to_owned),
            ));
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                bail!("mock exhausted");
            }
            Ok(responses.remove(0))
        }
    }

    fn item() -> AnalysisItem {
        AnalysisItem {
            region: "cn".into(),
            source_psm: "a.b".into(),
            source_method: "E".into(),
            target_psm: "c.d".into(),
            target_method: "F".into(),
            git_url: "g".into(),
            git_branch: "master".into(),
        }
    }

    #[tokio::test]
    async fn submit_returns_task_id() {
        let transport = MockTransport::new(vec![serde_json::json!({
            "created_items": [{"task_id": "dep-1"}],
            "rejected_inputs": [],
            "resolved_count": 1,
        })]);
        let id = submit(&transport, &item(), None).await.unwrap();
        assert_eq!(id, "dep-1");
        let calls = transport.calls.lock().unwrap();
        assert_eq!(calls[0].0, "POST");
        assert_eq!(calls[0].1, TASKS_SUFFIX);
        assert!(calls[0].3.is_some());
    }

    #[tokio::test]
    async fn submit_identity_sends_v3_request_id_and_identity_body() {
        let transport = MockTransport::new(vec![serde_json::json!({
            "created_items": [{"task_id": "dep-9"}],
            "rejected_inputs": [],
            "resolved_count": 1,
        })]);
        let id = submit_identity(&transport, 1983, 33848).await.unwrap();
        assert_eq!(id, "dep-9");
        let calls = transport.calls.lock().unwrap();
        let (_, _, headers, body) = &calls[0];
        // The identity-only body carries no five-tuple; the v3 request id is
        // passed via the x-request-id header rather than the body.
        let body: Value = serde_json::from_str(body.as_ref().unwrap()).unwrap();
        assert_eq!(
            body["items"][0],
            serde_json::json!({"link_id": 1983, "record_id": 33848})
        );
        assert!(body.get("source_sync_plan_fingerprint").is_none());
        let rid = headers
            .iter()
            .find(|(k, _)| k == "x-request-id")
            .map(|(_, v)| v.as_str())
            .unwrap();
        assert!(rid.starts_with(V3_REQUEST_PREFIX));
        assert_eq!(rid.len(), V3_REQUEST_PREFIX.len() + 40);
    }

    #[tokio::test]
    async fn poll_transitions_active_to_success() {
        let transport = MockTransport::new(vec![
            serde_json::json!({"task_id":"dep-1","status":"running"}),
            serde_json::json!({
                "task_id":"dep-1","status":"success","error_message":"",
                "result_depend_type":"weak",
                "result_payload":{"depend_type":"weak","status":"done","summary":"s"},
            }),
        ]);
        let cancel = CancellationToken::new();
        let terminal = poll_to_terminal(
            &transport,
            "dep-1",
            &cancel,
            Some(Duration::from_millis(1)),
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap();
        assert!(matches!(terminal, Terminal::Done(_)));
    }

    #[tokio::test]
    async fn poll_returns_failed_terminal() {
        let transport = MockTransport::new(vec![serde_json::json!({
            "task_id":"dep-1","status":"failed","error_message":"workflow blew up",
        })]);
        let cancel = CancellationToken::new();
        let terminal = poll_to_terminal(
            &transport,
            "dep-1",
            &cancel,
            Some(Duration::from_millis(1)),
            Some(Duration::from_secs(5)),
        )
        .await
        .unwrap();
        match terminal {
            Terminal::Failed(task) => {
                assert_eq!(task["status"], "failed");
            }
            _ => panic!("expected failed terminal"),
        }
    }

    #[tokio::test]
    async fn poll_honours_pre_cancelled_token() {
        let transport = MockTransport::new(vec![]);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = poll_to_terminal(
            &transport,
            "dep-1",
            &cancel,
            Some(Duration::from_millis(1)),
            Some(Duration::from_secs(5)),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("dep-1"));
    }
}

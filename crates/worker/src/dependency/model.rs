//! Pure data types and parsing for the viking strong/weak dependency flow.
//!
//! No I/O here: request bodies are built and responses are classified by
//! pure functions so the transport boundary (`cli.rs`) can be mocked in
//! tests. The wire contract mirrors the VTA trigger endpoint
//! `POST /api/v1/viking-test-agent/dependency-analysis/tasks`.

use serde_json::{json, Value};

/// The one analysis five-tuple plus the repo coordinates the agent analyzes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisItem {
    pub region: String,
    pub source_psm: String,
    pub source_method: String,
    pub target_psm: String,
    pub target_method: String,
    pub git_url: String,
    pub git_branch: String,
}

impl AnalysisItem {
    fn as_json(&self) -> Value {
        json!({
            "region": self.region,
            "source_psm": self.source_psm,
            "source_method": self.source_method,
            "target_psm": self.target_psm,
            "target_method": self.target_method,
            "git_url": self.git_url,
            "git_branch": self.git_branch,
        })
    }
}

/// Build the `POST .../dependency-analysis/tasks` body. `workflow_override`
/// is omitted unless the caller pins a specific workflow version.
pub fn build_create_body(item: &AnalysisItem, workflow_override: Option<&str>) -> Value {
    let mut body = json!({ "items": [item.as_json()] });
    if let Some(version) = workflow_override.filter(|v| !v.trim().is_empty()) {
        body["workflow_version_override"] = json!(version);
    }
    body
}

/// Extract the created child task id from a create response. A non-empty
/// `rejected_inputs` list (capacity/admission rejection) or a missing
/// `task_id` is a hard error — the caller must not resubmit blindly.
pub fn parse_created_task_id(resp: &Value) -> Result<String, String> {
    let rejected = resp.get("rejected_inputs");
    if let Some(rejected) =
        rejected.filter(|v| !v.is_array() || !v.as_array().is_some_and(|a| a.is_empty()))
    {
        return Err(format!("dependency task rejected on intake: {rejected}"));
    }
    let items = resp
        .get("created_items")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("create response missing created_items: {resp}"))?;
    if items.len() != 1 {
        return Err(format!(
            "create response accepted {} items, expected 1",
            items.len()
        ));
    }
    items[0]
        .get("task_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("create response omitted task_id: {}", items[0]))
}

/// Lifecycle phase of a dependency task, derived from its status string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusPhase {
    /// Reached a successful terminal state.
    Success,
    /// Reached a failed/cancelled terminal state.
    Failed,
    /// Still running (or an unrecognized status — keep polling until timeout).
    Active,
}

const SUCCESS_STATUSES: &[&str] = &["success", "succeeded", "completed"];
const FAILED_STATUSES: &[&str] = &["failed", "error", "canceled", "cancelled"];

/// Classify a task status. Unknown values map to [`StatusPhase::Active`] so a
/// newly-introduced status string never causes a premature failure; the poll
/// timeout is the backstop.
pub fn classify_status(status: &str) -> StatusPhase {
    let status = status.trim().to_ascii_lowercase();
    if SUCCESS_STATUSES.contains(&status.as_str()) {
        StatusPhase::Success
    } else if FAILED_STATUSES.contains(&status.as_str()) {
        StatusPhase::Failed
    } else {
        StatusPhase::Active
    }
}

/// Legal strong/weak verdicts returned by the analysis agent.
pub const RESULT_TYPES: &[&str] = &["strong", "strong_to_weak", "weak", "unknown"];

/// The validated successful result, flattened for how.md rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisResult {
    pub task_id: String,
    pub status: String,
    pub depend_type: String,
    pub summary: String,
    pub pair: Value,
}

/// Validate and flatten a successful task readback. Fail-closed: a success
/// terminal state that lacks the depend-type/summary proof is an error so an
/// incomplete success is never appended to how.md.
pub fn parse_result(task: &Value) -> Result<AnalysisResult, String> {
    let task_id = task
        .get("task_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "terminal task missing task_id".to_string())?
        .to_string();
    let status = task
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if classify_status(&status) != StatusPhase::Success {
        return Err(format!("{task_id}: not a success status: {status}"));
    }
    if !task
        .get("error_message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .is_empty()
    {
        return Err(format!("{task_id}: success status carries error_message"));
    }
    let depend_type = task
        .get("result_depend_type")
        .and_then(Value::as_str)
        .filter(|s| RESULT_TYPES.contains(&s.to_ascii_lowercase().as_str()))
        .map(|s| s.to_ascii_lowercase())
        .ok_or_else(|| format!("{task_id}: missing/invalid result_depend_type"))?;
    let payload = task
        .get("result_payload")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{task_id}: missing result_payload"))?;
    let payload_depend = payload
        .get("depend_type")
        .and_then(Value::as_str)
        .unwrap_or("");
    if payload_depend.to_ascii_lowercase() != depend_type {
        return Err(format!(
            "{task_id}: result_payload.depend_type ({payload_depend}) != {depend_type}"
        ));
    }
    let summary = payload
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{task_id}: result_payload.summary missing/empty"))?
        .to_string();
    let pair = json!({
        "region": task.get("region").and_then(Value::as_str).unwrap_or(""),
        "source_psm": task.get("source_psm").and_then(Value::as_str).unwrap_or(""),
        "source_method": task.get("source_method").and_then(Value::as_str).unwrap_or(""),
        "target_psm": task.get("target_psm").and_then(Value::as_str).unwrap_or(""),
        "target_method": task.get("target_method").and_then(Value::as_str).unwrap_or(""),
    });
    Ok(AnalysisResult {
        task_id,
        status,
        depend_type,
        summary,
        pair,
    })
}

/// Best-effort failure reason from a terminal-failed readback.
pub fn failure_reason(task: &Value) -> String {
    let status = task
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let error = task
        .get("error_message")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("analysis failed without an error message");
    format!("status={status}: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> AnalysisItem {
        AnalysisItem {
            region: "cn".into(),
            source_psm: "a.b.c".into(),
            source_method: "Entry".into(),
            target_psm: "x.y.z".into(),
            target_method: "Callee".into(),
            git_url: "git@code.byted.org:team/repo.git".into(),
            git_branch: "master".into(),
        }
    }

    #[test]
    fn create_body_carries_seven_fields_and_optional_override() {
        let body = build_create_body(&item(), None);
        let it = &body["items"][0];
        for field in [
            "region",
            "source_psm",
            "source_method",
            "target_psm",
            "target_method",
            "git_url",
            "git_branch",
        ] {
            assert!(it.get(field).is_some(), "missing {field}");
        }
        assert!(body.get("workflow_version_override").is_none());

        let with = build_create_body(&item(), Some("267"));
        assert_eq!(with["workflow_version_override"], json!("267"));
        // Blank override is dropped.
        let blank = build_create_body(&item(), Some("   "));
        assert!(blank.get("workflow_version_override").is_none());
    }

    #[test]
    fn created_task_id_parsed_or_rejected() {
        let ok = json!({"created_items":[{"task_id":"dep-123"}],"rejected_inputs":[],"resolved_count":1});
        assert_eq!(parse_created_task_id(&ok).unwrap(), "dep-123");

        let rejected = json!({"created_items":[],"rejected_inputs":[{"reason":"capacity"}]});
        assert!(parse_created_task_id(&rejected).is_err());

        let no_id = json!({"created_items":[{}]});
        assert!(parse_created_task_id(&no_id).is_err());
    }

    #[test]
    fn status_classification() {
        for s in ["success", "SUCCEEDED", " completed "] {
            assert_eq!(classify_status(s), StatusPhase::Success);
        }
        for s in ["failed", "error", "canceled", "cancelled"] {
            assert_eq!(classify_status(s), StatusPhase::Failed);
        }
        for s in [
            "pending",
            "dispatching",
            "submitted",
            "running",
            "some-new-state",
        ] {
            assert_eq!(classify_status(s), StatusPhase::Active);
        }
    }

    fn success_task() -> Value {
        json!({
            "task_id": "dep-9",
            "status": "success",
            "error_message": "",
            "region": "cn",
            "source_psm": "a.b.c",
            "source_method": "Entry",
            "target_psm": "x.y.z",
            "target_method": "Callee",
            "result_depend_type": "strong",
            "result_payload": {
                "depend_type": "strong",
                "status": "done",
                "summary": "entry 同步调用 Callee，失败即阻断",
            },
        })
    }

    #[test]
    fn parse_result_accepts_valid_and_rejects_incomplete() {
        let parsed = parse_result(&success_task()).unwrap();
        assert_eq!(parsed.depend_type, "strong");
        assert_eq!(parsed.task_id, "dep-9");

        let mut bad = success_task();
        bad["result_payload"]["summary"] = json!("  ");
        assert!(parse_result(&bad).is_err());

        let mut mismatch = success_task();
        mismatch["result_payload"]["depend_type"] = json!("weak");
        assert!(parse_result(&mismatch).is_err());

        let mut failed = success_task();
        failed["status"] = json!("failed");
        assert!(parse_result(&failed).is_err());

        let mut with_error = success_task();
        with_error["error_message"] = json!("boom");
        assert!(parse_result(&with_error).is_err());
    }
}

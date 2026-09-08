//! The `dependency_analysis` session tool.
//!
//! Registered only for the `dependency-analysis` persona (see `mod.rs`).
//! `analyze` submits one five-tuple, polls the hosted workflow to a terminal
//! state, and appends the validated verdict to the persona's how.md pool.
//! `status` is a non-mutating readback used to resume a long analysis.

use super::append;
use super::cli::{CliTransport, VikingTransport};
use super::flow;
use super::model::{self, AnalysisItem};
use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{Tool, ToolContext, ToolOutput};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub struct DependencyTool {
    transport: Arc<dyn VikingTransport>,
}

impl DependencyTool {
    pub fn new(transport: Arc<dyn VikingTransport>) -> Self {
        Self { transport }
    }

    /// Build the production transport from the process environment. Fails
    /// clearly when no issued token is present.
    pub fn from_env() -> Result<Self> {
        Ok(Self::new(Arc::new(CliTransport::from_env()?)))
    }
}

fn item_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "region": {"type": "string", "description": "部署 region，如 cn / i18n"},
            "source_psm": {"type": "string", "description": "调用方服务 PSM"},
            "source_method": {"type": "string", "description": "入口方法名"},
            "target_psm": {"type": "string", "description": "被调方服务 PSM"},
            "target_method": {"type": "string", "description": "被调方法名；HTTP 接口写成 'GET /path'"},
            "git_url": {"type": "string", "description": "被分析仓库 git 地址"},
            "git_branch": {"type": "string", "description": "被分析仓库分支"}
        },
        "required": ["region", "source_psm", "source_method", "target_psm", "target_method", "git_url", "git_branch"],
        "additionalProperties": false
    })
}

#[async_trait]
impl Tool for DependencyTool {
    fn name(&self) -> &str {
        "dependency_analysis"
    }

    fn description(&self) -> &str {
        "Run a viking strong/weak dependency (强弱依赖) analysis and persist the verdict to this persona's how.md knowledge pool. Action 'analyze': either pass 'anchor' {link_id, record_id} (a JianYingQA source-sync dryrun anchor — the supported intake while the scheduler is off; the server resolves the full caller/callee pair), or pass 'item' (region, source_psm, source_method, target_psm, target_method, git_url, git_branch) when open intake is enabled; then polls to terminal and appends the verdict. Action 'status' with a 'task_id' reads back an already-submitted analysis without resubmitting (it does not write)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["analyze", "status"]},
                "task_id": {"type": "string", "description": "status 必填；analyze 可选（传入则跳过提交，直接轮询该任务）"},
                "anchor": {
                    "type": "object",
                    "description": "JianYingQA 源同步 dryrun 锚点（scheduler 关闭时唯一可用的提交方式）",
                    "properties": {
                        "link_id": {"type": "integer", "minimum": 1},
                        "record_id": {"type": "integer", "minimum": 1}
                    },
                    "required": ["link_id", "record_id"],
                    "additionalProperties": false
                },
                "item": item_schema(),
                "workflow_version_override": {"type": "string", "description": "可选，固定工作流版本（仅 item 提交路径使用）"}
            },
            "required": ["action"],
            "allOf": [
                {"if": {"properties": {"action": {"const": "status"}}, "required": ["action"]},
                 "then": {"required": ["task_id"]}},
                {"if": {"properties": {"action": {"const": "analyze"},
                                     "not": {"required": ["task_id"]}}, "required": ["action"]},
                 "then": {"anyOf": [{"required": ["anchor"]}, {"required": ["item"]}]}}
            ]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let action = input.get("action").and_then(Value::as_str).unwrap_or("");
        match action {
            "analyze" => self.analyze(&input, ctx).await,
            "status" => self.status(&input).await,
            other => Ok(ToolOutput::err(format!("unsupported action: {other}"))),
        }
    }
}

impl DependencyTool {
    fn parse_item(input: &Value) -> Result<AnalysisItem, String> {
        let item = input
            .get("item")
            .and_then(Value::as_object)
            .ok_or_else(|| "analyze requires an 'item' object".to_string())?;
        let str_field = |key: &str| -> Result<String, String> {
            item.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| format!("item.{key} is required and must be a non-empty string"))
        };
        Ok(AnalysisItem {
            region: str_field("region")?,
            source_psm: str_field("source_psm")?,
            source_method: str_field("source_method")?,
            target_psm: str_field("target_psm")?,
            target_method: str_field("target_method")?,
            git_url: str_field("git_url")?,
            git_branch: str_field("git_branch")?,
        })
    }

    async fn analyze(&self, input: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let override_version = input
            .get("workflow_version_override")
            .and_then(Value::as_str);

        // A supplied task_id skips submit (idempotent resume of a prior run).
        let task_id = match input
            .get("task_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            Some(id) => id.to_string(),
            None => {
                if let Some(anchor) = input.get("anchor").and_then(Value::as_object) {
                    let link_id = anchor
                        .get("link_id")
                        .and_then(Value::as_u64)
                        .filter(|v| *v > 0)
                        .ok_or_else(|| {
                            anyhow::anyhow!("anchor.link_id must be a positive integer")
                        })?;
                    let record_id = anchor
                        .get("record_id")
                        .and_then(Value::as_u64)
                        .filter(|v| *v > 0)
                        .ok_or_else(|| {
                            anyhow::anyhow!("anchor.record_id must be a positive integer")
                        })?;
                    flow::submit_identity(self.transport.as_ref(), link_id, record_id).await?
                } else {
                    let item = Self::parse_item(input).map_err(anyhow::Error::msg)?;
                    flow::submit(self.transport.as_ref(), &item, override_version).await?
                }
            }
        };

        // Dropping this future on session interrupt stops the poll; the VTA
        // task keeps running server-side and can be resumed via `status`.
        let cancel = CancellationToken::new();
        let terminal =
            flow::poll_to_terminal(self.transport.as_ref(), &task_id, &cancel, None, None).await?;

        match terminal {
            flow::Terminal::Done(task) => {
                let result = model::parse_result(&task).map_err(anyhow::Error::msg)?;
                let appended = append::append_result(&ctx.agent, &result)
                    .map_err(|e| anyhow::anyhow!("how.md append failed: {e}"))?;
                Ok(ToolOutput::ok(
                    json!({
                        "task_id": result.task_id,
                        "depend_type": result.depend_type,
                        "summary": result.summary,
                        "appended_version": appended,
                        "persona": ctx.agent,
                    })
                    .to_string(),
                ))
            }
            flow::Terminal::Failed(task) => Ok(ToolOutput::err(format!(
                "dependency analysis {} failed: {}",
                task_id,
                model::failure_reason(&task)
            ))),
        }
    }

    async fn status(&self, input: &Value) -> Result<ToolOutput> {
        let task_id = input
            .get("task_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("status requires a non-empty task_id"))?;
        let task = flow::readback(self.transport.as_ref(), task_id).await?;
        let status = task.get("status").and_then(Value::as_str).unwrap_or("");
        match model::classify_status(status) {
            model::StatusPhase::Success => {
                let result = model::parse_result(&task).map_err(anyhow::Error::msg)?;
                Ok(ToolOutput::ok(
                    json!({
                        "task_id": result.task_id,
                        "phase": "success",
                        "depend_type": result.depend_type,
                        "summary": result.summary,
                        "note": "status does not append to how.md; run analyze with this task_id to persist",
                    })
                    .to_string(),
                ))
            }
            model::StatusPhase::Failed => Ok(ToolOutput::err(format!(
                "dependency analysis {task_id} failed: {}",
                model::failure_reason(&task)
            ))),
            model::StatusPhase::Active => Ok(ToolOutput::ok(
                json!({"task_id": task_id, "phase": "active", "status": status}).to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MockTransport {
        responses: Mutex<Vec<Value>>,
    }
    impl MockTransport {
        fn new(responses: Vec<Value>) -> Self {
            Self {
                responses: Mutex::new(responses),
            }
        }
    }
    #[async_trait]
    impl VikingTransport for MockTransport {
        async fn request(
            &self,
            _method: &str,
            _suffix: &str,
            _headers: &[(String, String)],
            _query: &[(String, String)],
            _body: Option<&str>,
        ) -> Result<Value> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                anyhow::bail!("mock exhausted");
            }
            Ok(responses.remove(0))
        }
    }

    fn ctx() -> ToolContext {
        ToolContext {
            session_id: "s1".into(),
            message_id: "m1".into(),
            agent: "dependency-analysis".into(),
            working_dir: std::env::temp_dir(),
            max_output: 1024,
            proxy: None,
            tools_path: None,
            extra_env: vec![],
        }
    }

    fn analyze_input() -> Value {
        json!({
            "action": "analyze",
            "item": {
                "region": "cn",
                "source_psm": "a.b",
                "source_method": "Entry",
                "target_psm": "c.d",
                "target_method": "Callee",
                "git_url": "git@code.byted.org:t/r.git",
                "git_branch": "master"
            }
        })
    }

    #[test]
    fn schema_requires_item_for_analyze_and_task_for_status() {
        let tool = DependencyTool::new(Arc::new(MockTransport::default()));
        let schema = tool.parameters();
        assert_eq!(
            schema["properties"]["action"]["enum"],
            json!(["analyze", "status"])
        );
        let all_of = schema["allOf"].as_array().unwrap();
        assert_eq!(all_of.len(), 2);
    }

    #[test]
    fn parse_item_validates_required_fields() {
        let mut input = analyze_input();
        assert!(DependencyTool::parse_item(&input).is_ok());
        input["item"]["source_psm"] = json!("  ");
        assert!(DependencyTool::parse_item(&input).is_err());
        input["item"].as_object_mut().unwrap().remove("git_branch");
        assert!(DependencyTool::parse_item(&input).is_err());
    }

    #[tokio::test]
    async fn analyze_rejects_unknown_action() {
        let tool = DependencyTool::new(Arc::new(MockTransport::default()));
        let out = tool
            .execute(json!({"action": "frobnicate"}), &ctx())
            .await
            .unwrap();
        assert!(out.is_error);
    }

    #[tokio::test]
    async fn status_active_does_not_append() {
        let tool = DependencyTool::new(Arc::new(MockTransport::new(vec![
            json!({"task_id": "dep-1", "status": "running"}),
        ])));
        let out = tool
            .execute(json!({"action": "status", "task_id": "dep-1"}), &ctx())
            .await
            .unwrap();
        assert!(!out.is_error);
        assert!(out.content.contains("active"));
    }

    #[tokio::test]
    async fn status_failed_is_error() {
        let tool = DependencyTool::new(Arc::new(MockTransport::new(vec![
            json!({"task_id": "dep-1", "status": "failed", "error_message": "boom"}),
        ])));
        let out = tool
            .execute(json!({"action": "status", "task_id": "dep-1"}), &ctx())
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(out.content.contains("boom"));
    }
}

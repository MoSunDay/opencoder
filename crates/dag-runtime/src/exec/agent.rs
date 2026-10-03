//! Agent-step executor: run one step's prompt through the REAL session
//! runner on a fresh local session — the same public building blocks the
//! node-task executor composes (`resume_and_replay`, `session.cancel`,
//! `spawn_event_flusher`, `opencoder_session::run`).
//!
//! Cancellation arrives as the step's [`CancellationToken`] and is wired
//! straight into `session.cancel` BEFORE the run, so the runner's own
//! interrupt path converges the turn; no separate flag race is needed.
//! Transcript capture keeps a bounded tail (last ~8KB) and scans it for a
//! ```json fenced block to recover structured output.

use opencoder_core::message::now_ms;
use opencoder_dag::{StepKind, StepSpec};
use opencoder_store::{SessionMeta, TASK_TYPE_AGENT_STEP};
use tokio_util::sync::CancellationToken;

use super::{ExecDeps, StepCtx, StepResult};

pub async fn execute_agent_step(
    ctx: &StepCtx,
    deps: &ExecDeps,
    cancel: CancellationToken,
) -> StepResult {
    match super::how_copy::prepare(ctx) {
        Ok(_) => {}
        Err(error) => {
            return super::native::io::error_result(format!(
                "prepare local Agent resources: {error:#}"
            ))
        }
    }
    let mut selected = deps.clone();
    if let StepKind::Agent {
        model: Some(model), ..
    } = ctx.step.kind.executable()
    {
        selected.config.model = model.clone();
    }
    super::agent_runc::execute_agent_step_runc(ctx, &selected, cancel).await
}

pub(crate) fn step_agent_name(step: &StepSpec) -> String {
    match &step.kind {
        StepKind::Agent { agent, .. } => agent.clone().unwrap_or_else(|| "act".into()),
        _ => "act".into(),
    }
}

/// Prompt = step prompt + upstream context header + structured-output
/// instruction. The context is the same object a binary step receives as its
/// `context.json` input file in the step's read-only metadata directory.
pub(crate) fn build_prompt_with_knowledge(
    ctx: &StepCtx,
    knowledge: Option<&std::path::Path>,
) -> String {
    let prompt = match &ctx.step.kind {
        StepKind::Agent { prompt, .. } => prompt.clone(),
        _ => String::new(),
    };
    let context = serde_json::to_string_pretty(&ctx.context()).unwrap_or_else(|_| "{}".into());
    format!(
        "{}\n\n上游步骤输出（JSON）：\n{}\n\n{}\n\n如果本步骤需要产出结构化结果，请在最终回复的末尾追加一个 ```json 围栏代码块（fenced code block）包含该 JSON。",
        prompt,
        context,
        knowledge_hint(knowledge)
    )
}

/// The read-only knowledge-root hint appended to agent prompts (host-path
/// form: host-sandbox sessions read the node's real tree). The mount is
/// READ-ONLY — the prompt states the contract, the kernel/FsPerms enforces
/// it for sandboxed steps.
fn knowledge_hint(knowledge: Option<&std::path::Path>) -> String {
    let Some(root) = knowledge else {
        return String::new();
    };
    format!(
        "知识库（只读）：{} 可读取参考，但禁止写入或修改其中任何内容（git 操作请加 --no-optional-locks）；你的产物一律写入本步骤目录（OPENCODER_STEP_DIR）。",
        root.display()
    )
}

/// Persist a fresh local session row for this step (the node executor's
/// `create_local_meta` shape, pinned as an internal Agent step so it cannot
/// leak into the top-level Agent chat lane.
pub(crate) async fn create_session_meta(
    deps: &ExecDeps,
    step: &StepSpec,
    run_id: &str,
) -> anyhow::Result<String> {
    let agent = match &step.kind {
        StepKind::Agent { agent, .. } => agent.clone(),
        _ => anyhow::bail!("non-agent step dispatched to the agent executor"),
    };
    let id = ulid::Ulid::new().to_string();
    let now = now_ms();
    deps.store
        .create_session(&SessionMeta {
            kind: Some("dag".into()),
            id: id.clone(),
            title: Some(format!("dag/{}/{}", run_id, step.name)),
            agent: agent.or_else(|| Some("act".into())),
            model: Some(deps.config.model.clone()),
            autopilot_mode: None,
            workdir_hash: Some(opencoder_core::workdir_hash(&deps.workdir)),
            created_at: now,
            updated_at: now,
            summary: None,
            summary_seq: None,
            summary_images: vec![],
            handoff_seq: None,
            handoff_plan: None,
            skill: None,
            task_type: Some(TASK_TYPE_AGENT_STEP.into()),
            requirement: None,
        })
        .await?;
    Ok(id)
}

pub use opencoder_session::harness::output::extract_output_json_from;

/// Terminal decision by precedence: cancelled > error > done (the node
/// executor's `terminal_report`, step-flavored).
#[cfg(test)]
mod tests {
    use super::*;

    /// The json-fence scanner: last fence wins, prose around it is ignored,
    /// a bare JSON text still parses, garbage stays `None`.
    #[test]
    fn extracts_json_from_fence_or_whole_text() {
        let fenced = "analysis...\n```json\n{\"answer\": 1}\n```\ntail";
        assert_eq!(
            extract_output_json_from(fenced).unwrap()["answer"],
            serde_json::json!(1)
        );
        let two_fences = "```json\n{\"first\": true}\n```\nmore\n```json\n{\"second\": 2}\n```";
        assert_eq!(
            extract_output_json_from(two_fences).unwrap()["second"],
            serde_json::json!(2)
        );
        assert_eq!(
            extract_output_json_from("  {\"bare\": 3}  ").unwrap()["bare"],
            serde_json::json!(3)
        );
        assert!(extract_output_json_from("no structure here").is_none());
        // An unterminated fence still yields its body.
        assert_eq!(
            extract_output_json_from("```json\n{\"open\": 4}").unwrap()["open"],
            serde_json::json!(4)
        );
    }

    /// Bare JSON after narration (the no-fence contract form): the whole
    /// text is NOT valid JSON, so the pre-fix whole-text parse returned
    /// `None` here — the fallback scan is what recovers the object.
    #[test]
    fn extracts_bare_json_from_narration_tail() {
        let reply = "## 分析过程\n调用链定位到 handler，签名匹配，关键证据如下……（长叙述）\n最终结论：\n{\"depend_type\": \"strong\", \"analysis_report\": {\"调用链\": \"router->handler\"}}\n";
        let value = extract_output_json_from(reply).unwrap();
        assert_eq!(value["depend_type"], serde_json::json!("strong"));
        assert_eq!(
            value["analysis_report"]["调用链"],
            serde_json::json!("router->handler")
        );
        // The LAST top-level object wins when narration carries examples.
        let mixed = "示例 {\"not\": \"this\"} 与 {a: 占位} 说明。\n{\"final\": 7}";
        assert_eq!(
            extract_output_json_from(mixed).unwrap()["final"],
            serde_json::json!(7)
        );
    }

    /// A broken fence falls through to the tail scan instead of dropping the
    /// whole structured output; prose braces never shadow a real object.
    #[test]
    fn broken_fence_falls_back_to_tail_bare_json() {
        // The fence closes, but its body is not valid JSON: the fence branch
        // fails and the tail scan recovers the bare object after it.
        let reply = "```json\n{\"broken\":\n```\n结论：\n{\"depend_type\": \"weak\"}";
        assert_eq!(
            extract_output_json_from(reply).unwrap()["depend_type"],
            serde_json::json!("weak")
        );
        // Braces inside JSON strings are inert; the scan still balances.
        let strings = "{\"code\": \"} { \\n { \"} ... 混入叙述 {\"answer\": 9}";
        assert_eq!(
            extract_output_json_from(strings).unwrap()["answer"],
            serde_json::json!(9)
        );
        // Unterminated object at the very end yields nothing.
        assert!(extract_output_json_from("结论 {\"open\": 1").is_none());
    }

    /// The fallback only scans the 8KB tail: objects buried earlier in a
    /// huge reply are out of scope, objects at the end are always reached.
    #[test]
    fn bare_json_scan_is_bounded_to_the_tail() {
        let late = format!("{}\n{{\"tail_only\": true}}", "x".repeat(9000));
        assert!(extract_output_json_from(&late).unwrap()["tail_only"]
            .as_bool()
            .unwrap());
        let early = format!("{{\"head_only\": true}}\n{}", "y".repeat(9000));
        assert!(extract_output_json_from(&early).is_none());
    }
}

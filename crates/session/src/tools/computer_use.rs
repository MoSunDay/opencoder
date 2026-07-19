//! `computer_use` tool: runs the native perceive->act loop
//! ([`opencoder_core::ComputerUseLoop`], distilled from cua) against an
//! executor. This first version replays a caller-supplied action plan through a
//! [`RecordingExecutor`] and returns the resulting action trace, so the loop is
//! exercised end-to-end and is deterministic/testable without a live sandbox.
//!
//! Live LLM-driven perceive->act against an Anthropic/OpenAI computer-use
//! sandbox is the next milestone: the loop + [`ComputerUseExecutor`] abstraction
//! in core are ready to receive a real provider executor.

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{
    json, tool::truncate_output, ComputerAction, ComputerUseLoop,
    LoopOutcome, RecordingExecutor, Tool, ToolContext, ToolOutput,
};
use serde_json::Value;

pub struct ComputerUseTool;

#[async_trait]
impl Tool for ComputerUseTool {
    fn name(&self) -> &str {
        "computer_use"
    }
    fn description(&self) -> &str {
        "Drive a computer-use agent loop (perceive -> act -> repeat). Supply a `prompt` describing the task and an optional `actions` plan to replay; returns the executed action trace and the loop outcome (done / max_steps). First version replays the plan in a sandbox executor; live provider-driven execution is in progress."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert("prompt".into(), json::prop_str("The high-level task to perform."));
        props.insert(
            "actions".into(),
            serde_json::json!({
                "type": "array",
                "description": "Optional ordered action plan to replay, e.g. [{\"type\":\"click\",\"x\":120,\"y\":340},{\"type\":\"type\",\"text\":\"hi\"},{\"type\":\"done\"}].",
                "items": { "type": "object" }
            }),
        );
        props.insert(
            "max_steps".into(),
            serde_json::json!({ "type": "integer", "description": "Step budget (default 25)." }),
        );
        json::object_schema(Value::Object(props), &["prompt"])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let prompt = input
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if prompt.is_empty() {
            return Ok(ToolOutput::err("prompt is required"));
        }
        let max_steps = input
            .get("max_steps")
            .and_then(|v| v.as_u64())
            .unwrap_or(25)
            .clamp(1, 200) as usize;

        let mut plan: Vec<ComputerAction> = Vec::new();
        if let Some(arr) = input.get("actions").and_then(|v| v.as_array()) {
            for a in arr {
                match serde_json::from_value::<ComputerAction>(a.clone()) {
                    Ok(act) => plan.push(act),
                    Err(e) => return Ok(ToolOutput::err(format!("invalid action: {e}"))),
                }
            }
        }

        let exec = RecordingExecutor::default();
        let mut idx = 0usize;
        let outcome = ComputerUseLoop::new(&exec, max_steps)
            .run(|_| {
                let a = plan.get(idx).cloned();
                idx += 1;
                a
            })
            .await?;

        let recorded = exec.actions.lock().unwrap().clone();
        let mut report = String::new();
        report.push_str(&format!("task: {prompt}\n"));
        report.push_str(&format!(
            "outcome: {}\n",
            match outcome.0 {
                LoopOutcome::Done => "done",
                LoopOutcome::MaxStepsReached => "max_steps_reached",
            }
        ));
        report.push_str(&format!("final: {}\n", outcome.1.text));
        report.push_str(&format!("actions_executed: {}\n", recorded.len()));
        for (i, a) in recorded.iter().enumerate() {
            report.push_str(&format!(
                "  [{}] {} {}\n",
                i,
                a.kind,
                serde_json::to_string(&a.fields).unwrap_or_default()
            ));
        }
        if plan.is_empty() {
            report.push_str(
                "note: no `actions` plan supplied. Live LLM-driven perceive->act against a \
                 provider sandbox is the next milestone; provide an `actions` plan to exercise \
                 the loop deterministically.",
            );
        }
        Ok(truncate_output(report, ctx.max_output))
    }
}

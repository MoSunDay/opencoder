use std::process::Stdio;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use opencode_core::{json, Tool, ToolContext, ToolOutput};
use serde_json::Value;

pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str { "bash" }
    fn description(&self) -> &str {
        "Executes a bash command in the session working directory and returns stdout+stderr. Use for git, builds, tests, running scripts. Commands run non-interactively."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert("command".into(), json::prop_str("The bash command to execute."));
        props.insert("workdir".into(), json::prop_str("Optional working directory override."));
        props.insert("timeout".into(), serde_json::json!({ "type": "number", "description": "Optional timeout in seconds (default 120)." }));
        json::object_schema(Value::Object(props), &["command"])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");
        if command.trim().is_empty() {
            return Ok(ToolOutput::err("empty command"));
        }
        let workdir = input
            .get("workdir")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());
        let timeout_secs = input.get("timeout").and_then(|v| v.as_u64()).unwrap_or(120);

        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-lc").arg(command).current_dir(&workdir)
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .kill_on_drop(true);

        let output = match tokio::time::timeout(Duration::from_secs(timeout_secs), cmd.output()).await {
            Ok(o) => o?,
            Err(_) => return Ok(ToolOutput::err(format!("command timed out after {timeout_secs}s"))),
        };
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let code = output.status.code().unwrap_or(-1);
        let mut combined = String::new();
        if !stdout.is_empty() { combined.push_str(&stdout); }
        if !stderr.is_empty() {
            if !combined.is_empty() { combined.push('\n'); }
            combined.push_str("[stderr]\n");
            combined.push_str(&stderr);
        }
        if combined.is_empty() { combined.push_str("(no output)"); }
        combined.push_str(&format!("\n[exit code: {code}]"));
        let is_error = code != 0;
        if is_error {
            Ok(ToolOutput::err(combined))
        } else {
            Ok(opencode_core::tool::truncate_output(combined, ctx.max_output))
        }
    }
}

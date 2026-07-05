use anyhow::Result;
use async_trait::async_trait;
use opencode_core::{json, Tool, ToolContext, ToolOutput};
use serde_json::Value;

#[derive(Default)]
pub struct PlanExitTool;

#[async_trait]
impl Tool for PlanExitTool {
    fn name(&self) -> &str { "plan_exit" }
    fn description(&self) -> &str {
        "Exit plan mode and switch to act (execution) mode. Writes the plan markdown to .opencode/plans/ and signals the runner to switch agents. Call this once the plan is complete."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert("plan".into(), json::prop_str("The full plan content in markdown."));
        props.insert("filename".into(), json::prop_str("Optional plan file name (without extension)."));
        json::object_schema(Value::Object(props), &["plan"])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let plan = input.get("plan").and_then(|v| v.as_str()).unwrap_or("");
        let name = input.get("filename").and_then(|v| v.as_str()).unwrap_or("plan");
        let dir = ctx.working_dir.join(".opencode").join("plans");
        tokio::fs::create_dir_all(&dir).await.ok();
        let file = dir.join(format!("{}.md", sanitize(name)));
        if let Err(e) = tokio::fs::write(&file, plan).await {
            return Ok(ToolOutput::err(format!("write plan: {e}")));
        }
        Ok(ToolOutput::ok(format!("plan written to {}; switching to act mode", file.display())))
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

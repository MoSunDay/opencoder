use anyhow::Result;
use async_trait::async_trait;
use opencode_core::{json, Tool, ToolContext, ToolOutput};
use serde_json::Value;

pub struct ListTool;

#[async_trait]
impl Tool for ListTool {
    fn name(&self) -> &str {
        "ls"
    }
    fn description(&self) -> &str {
        "Lists the contents of a directory. Returns names with a trailing '/' for directories."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert(
            "path".into(),
            json::prop_str("Optional directory path (defaults to working dir)."),
        );
        json::object_schema(Value::Object(props), &[])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let base = input
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| ctx.working_dir.display().to_string());
        let path = std::path::Path::new(&base);
        let entries = match std::fs::read_dir(path) {
            Ok(e) => e,
            Err(e) => return Ok(ToolOutput::err(format!("ls {}: {e}", path.display()))),
        };
        let mut names: Vec<String> = Vec::new();
        for entry in entries.flatten() {
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let name = entry.file_name().to_string_lossy().to_string();
            names.push(if is_dir { format!("{name}/") } else { name });
        }
        names.sort();
        if names.is_empty() {
            return Ok(ToolOutput::ok("(empty)"));
        }
        Ok(opencode_core::tool::truncate_output(
            names.join("\n"),
            ctx.max_output,
        ))
    }
}

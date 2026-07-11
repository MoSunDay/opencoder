use anyhow::Result;
use async_trait::async_trait;
use opencode_core::{json, Tool, ToolContext, ToolOutput};
use serde_json::Value;

pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }
    fn description(&self) -> &str {
        "Fast file pattern matching. Returns file paths matching the glob pattern (e.g. \"**/*.rs\")."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert(
            "pattern".into(),
            json::prop_str("Glob pattern, e.g. \"src/**/*.rs\"."),
        );
        props.insert("path".into(), json::prop_str("Optional base directory."));
        json::object_schema(Value::Object(props), &["pattern"])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let pattern = input.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
        let base = input
            .get("path")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());
        let full_pattern = if pattern.starts_with('/') {
            pattern.to_string()
        } else {
            format!("{}/{}", base.display(), pattern)
        };
        let mut paths: Vec<String> = glob::glob(&full_pattern)
            .map_err(|e| anyhow::anyhow!("invalid glob: {e}"))?
            .filter_map(|r| r.ok())
            .map(|p| p.display().to_string())
            .collect();
        paths.sort();
        if paths.is_empty() {
            return Ok(ToolOutput::ok("no matches"));
        }
        let out = paths
            .iter()
            .take(500)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        Ok(opencode_core::tool::truncate_output(out, ctx.max_output))
    }
}

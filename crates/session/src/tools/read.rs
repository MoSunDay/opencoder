use anyhow::Result;
use async_trait::async_trait;
use opencode_core::{json, Tool, ToolContext, ToolOutput};
use serde_json::Value;

pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }
    fn description(&self) -> &str {
        "Reads a UTF-8 text file from the filesystem. Supports optional line offset and limit. Returns the file content."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert(
            "path".into(),
            json::prop_str("Path to the file to read, relative to the working directory."),
        );
        props.insert("offset".into(), serde_json::json!({ "type": "integer", "description": "Starting 1-based line number (optional)." }));
        props.insert("limit".into(), serde_json::json!({ "type": "integer", "description": "Max number of lines to read (optional)." }));
        json::object_schema(Value::Object(props), &["path"])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let full = resolve(ctx, path);
        let content = match tokio::fs::read_to_string(&full).await {
            Ok(c) => c,
            Err(e) => return Ok(ToolOutput::err(format!("read {}: {e}", full.display()))),
        };
        let offset = input
            .get("offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .max(1) as usize;
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        let lines: Vec<&str> = content.lines().collect();
        let start = (offset - 1).min(lines.len());
        let end = match limit {
            Some(n) => (start + n).min(lines.len()),
            None => lines.len(),
        };
        let mut out = String::new();
        for (i, line) in lines[start..end].iter().enumerate() {
            out.push_str(&format!("{:>5}: {}\n", start + i + 1, line));
        }
        if out.is_empty() {
            out.push_str("(empty)");
        }
        Ok(opencode_core::tool::truncate_output(out, ctx.max_output))
    }
}

pub(crate) fn resolve(ctx: &ToolContext, path: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        ctx.working_dir.join(p)
    }
}

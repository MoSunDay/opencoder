use anyhow::Result;
use async_trait::async_trait;
use opencode_core::{json, Tool, ToolContext, ToolOutput};
use serde_json::Value;

pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }
    fn description(&self) -> &str {
        "Creates or overwrites a file with the given content. Creates parent directories. Use for new files only; prefer edit for modifying existing files."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert("path".into(), json::prop_str("Path of the file to write."));
        props.insert(
            "content".into(),
            json::prop_str("Full content to write to the file."),
        );
        json::object_schema(Value::Object(props), &["path", "content"])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let content = input.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let full = super::read::resolve(ctx, path);
        if let Some(parent) = full.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        if let Err(e) = tokio::fs::write(&full, content).await {
            return Ok(ToolOutput::err(format!("write {}: {e}", full.display())));
        }
        Ok(ToolOutput::ok(format!(
            "wrote {} ({} bytes)",
            full.display(),
            content.len()
        )))
    }
}

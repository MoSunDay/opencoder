use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{json, Tool, ToolContext, ToolOutput};
use serde_json::Value;

pub struct EditTool;

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }
    fn description(&self) -> &str {
        "Performs an exact string replacement in an existing file. old_string must match exactly once (unless replace_all). Fails if the match is not found or is ambiguous."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert("path".into(), json::prop_str("Path of the file to edit."));
        props.insert(
            "old_string".into(),
            json::prop_str("The exact text to replace."),
        );
        props.insert("new_string".into(), json::prop_str("The replacement text."));
        props.insert("replace_all".into(), serde_json::json!({ "type": "boolean", "description": "Replace every occurrence. Default false." }));
        json::object_schema(Value::Object(props), &["path", "old_string", "new_string"])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let old_string = input
            .get("old_string")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let new_string = input
            .get("new_string")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let replace_all = input
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let full = super::read::resolve(ctx, path);
        let content = match tokio::fs::read_to_string(&full).await {
            Ok(c) => c,
            Err(e) => return Ok(ToolOutput::err(format!("read {}: {e}", full.display()))),
        };
        if old_string.is_empty() {
            return Ok(ToolOutput::err("old_string is empty"));
        }
        let count = content.matches(old_string).count();
        if count == 0 {
            return Ok(ToolOutput::err("old_string not found in file"));
        }
        if count > 1 && !replace_all {
            return Ok(ToolOutput::err(format!(
                "old_string matches {count} times; set replace_all=true or add context"
            )));
        }
        let updated = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };
        if let Err(e) = tokio::fs::write(&full, &updated).await {
            return Ok(ToolOutput::err(format!("write {}: {e}", full.display())));
        }
        Ok(ToolOutput::ok(format!(
            "edited {} ({} replacement{})",
            full.display(),
            if replace_all { count } else { 1 },
            if count == 1 { "" } else { "s" }
        )))
    }
}

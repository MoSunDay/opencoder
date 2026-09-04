use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{json, Tool, ToolContext, ToolOutput};
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
        let max_output = ctx.max_output;

        let out = tokio::task::spawn_blocking(move || -> ToolOutput {
            let path = std::path::Path::new(&base);
            let entries = match std::fs::read_dir(path) {
                Ok(e) => e,
                Err(e) => return ToolOutput::err(format!("ls {}: {e}", path.display())),
            };
            let mut names: Vec<String> = Vec::new();
            for entry in entries.flatten() {
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                let name = entry.file_name().to_string_lossy().to_string();
                names.push(if is_dir { format!("{name}/") } else { name });
            }
            names.sort();
            if names.is_empty() {
                return ToolOutput::ok("(empty)");
            }
            opencoder_core::tool::truncate_output(names.join("\n"), max_output)
        })
        .await
        .unwrap_or_else(|e| ToolOutput::err(format!("ls task failed: {e}")));

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::Tool;
    use serde_json::json;
    use std::io::Write;

    fn ctx_for(dir: &tempfile::TempDir) -> ToolContext {
        ToolContext {
            session_id: "test".into(),
            message_id: "test".into(),
            agent: "explore".into(),
            working_dir: dir.path().to_path_buf(),
            max_output: 4096,
            proxy: None,
            tools_path: None,
        }
    }

    #[tokio::test]
    async fn ls_lists_directory_contents() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("file.txt")).unwrap();
        writeln!(f, "data").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();
        let ctx = ctx_for(&dir);
        let tool = ListTool;
        let out = tool
            .execute(json!({ "path": dir.path().display().to_string() }), &ctx)
            .await
            .unwrap();
        assert!(!out.is_error, "expected success, got: {}", out.content);
        assert!(
            out.content.contains("file.txt"),
            "expected file.txt, got: {}",
            out.content
        );
        assert!(
            out.content.contains("subdir/"),
            "expected subdir with trailing slash, got: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn ls_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx_for(&dir);
        let tool = ListTool;
        let out = tool
            .execute(json!({ "path": dir.path().display().to_string() }), &ctx)
            .await
            .unwrap();
        assert!(!out.is_error);
        assert_eq!(out.content, "(empty)");
    }

    #[tokio::test]
    async fn ls_nonexistent_path_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does_not_exist");
        let ctx = ctx_for(&dir);
        let tool = ListTool;
        let out = tool
            .execute(json!({ "path": missing.display().to_string() }), &ctx)
            .await
            .unwrap();
        assert!(out.is_error, "expected error for missing path");
    }
}

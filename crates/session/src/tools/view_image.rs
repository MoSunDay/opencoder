//! Read a local image file and return it inline so the vision model can
//! visually inspect it. For screenshots, diagrams, charts, photos. Supports
//! png/jpg/jpeg/gif/webp/bmp (and any format sniffable from magic bytes).

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{Tool, ToolContext, ToolOutput};
use serde_json::{json, Value};

pub struct ViewImageTool;

#[async_trait]
impl Tool for ViewImageTool {
    fn name(&self) -> &str {
        "view_image"
    }
    fn description(&self) -> &str {
        "Read a local image file (png/jpg/jpeg/gif/webp/bmp) and return it inline \
         so you can visually inspect it. Use for screenshots, diagrams, charts, \
         and photos. The image is sent directly to the model (not as text)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the image file. Relative paths resolve against the working directory."
                }
            },
            "required": ["path"]
        })
    }
    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
        if path.is_empty() {
            return Ok(ToolOutput::err("Missing required parameter: path."));
        }
        // Resolve the same way the `read` tool does (absolute or relative to workdir).
        let full = super::read::resolve(ctx, path);
        match std::fs::read(&full) {
            Ok(bytes) => {
                let data_uri = super::image_data::bytes_to_data_uri(&bytes);
                let kb = bytes.len() as f64 / 1024.0;
                Ok(ToolOutput::ok_with_images(
                    format!("Loaded image: {} ({:.1} KiB)", full.display(), kb),
                    vec![data_uri],
                ))
            }
            Err(e) => Ok(ToolOutput::err(format!(
                "view_image {}: {e}",
                full.display()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx(workdir: std::path::PathBuf) -> ToolContext {
        ToolContext {
            session_id: "test".into(),
            message_id: "test".into(),
            agent: "act".into(),
            working_dir: workdir,
            max_output: 4096,
            proxy: None,
        }
    }

    // 1x1 transparent PNG (smallest valid PNG with IHDR + IDAT + IEND).
    const PNG_BYTES: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
    ];

    #[tokio::test]
    async fn returns_image_inline_for_png() {
        let dir = tempfile::tempdir().unwrap();
        let img_path = dir.path().join("pixel.png");
        std::fs::write(&img_path, PNG_BYTES).unwrap();

        let tool = ViewImageTool;
        let input = json!({ "path": "pixel.png" });
        let out = tool.execute(input, &ctx(dir.path().to_path_buf())).await;

        let result = out.expect("execute must not error for a readable image");
        assert!(!result.is_error, "expected success, got: {}", result.content);
        assert!(
            !result.images.is_empty(),
            "image attachment must be present"
        );
        assert!(
            result.images[0].starts_with("data:image/png;base64,"),
            "expected png data uri, got: {}",
            result.images[0]
        );
    }

    #[tokio::test]
    async fn missing_path_returns_error_output() {
        let dir = tempfile::tempdir().unwrap();
        let tool = ViewImageTool;
        let out = tool
            .execute(json!({}), &ctx(dir.path().to_path_buf()))
            .await
            .unwrap();
        assert!(out.is_error, "missing path must be an error");
        assert!(out.images.is_empty(), "error must carry no images");
    }

    #[tokio::test]
    async fn empty_path_returns_error_output() {
        let dir = tempfile::tempdir().unwrap();
        let tool = ViewImageTool;
        let out = tool
            .execute(json!({ "path": "" }), &ctx(dir.path().to_path_buf()))
            .await
            .unwrap();
        assert!(out.is_error, "empty path must be an error");
    }

    #[tokio::test]
    async fn nonexistent_path_returns_error_output() {
        let dir = tempfile::tempdir().unwrap();
        let tool = ViewImageTool;
        let out = tool
            .execute(json!({ "path": "nope.png" }), &ctx(dir.path().to_path_buf()))
            .await
            .unwrap();
        assert!(out.is_error, "nonexistent path must be an error");
        assert!(
            out.content.contains("view_image"),
            "error text should reference the tool: {}",
            out.content
        );
    }
}

//! Shell-free executable tools registered through project CLI config.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use opencoder_core::{CliToolConfig, Tool, ToolContext, ToolOutput};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::{
    process::Command,
    time::{timeout, Duration},
};

use super::image_data::tool_image_to_data_uri;

const MAX_STDIO_BYTES: usize = 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;

pub struct RegisteredCliTool {
    config: CliToolConfig,
    description: String,
}

impl RegisteredCliTool {
    pub fn new(config: CliToolConfig, description: String) -> Result<Self> {
        if config.name.trim().is_empty() || config.executable.trim().is_empty() {
            return Err(anyhow!("registered CLI tool requires name and executable"));
        }
        if config.input_field.trim().is_empty() {
            return Err(anyhow!("registered CLI tool requires input_field"));
        }
        if !matches!(config.input_mode.as_str(), "field" | "json") {
            return Err(anyhow!(
                "registered CLI tool input_mode must be field or json"
            ));
        }
        Ok(Self {
            config,
            description,
        })
    }

    async fn images(&self, value: &Value, root: &Path) -> Result<Vec<String>> {
        let root = tokio::fs::canonicalize(root).await?;
        let mut images = Vec::new();
        for pointer in &self.config.image_path_pointers {
            let Some(path) = value.pointer(pointer).and_then(Value::as_str) else {
                continue;
            };
            let path = tokio::fs::canonicalize(path)
                .await
                .with_context(|| format!("registered CLI image is unavailable: {path}"))?;
            if !path.starts_with(&root) {
                return Err(anyhow!("registered CLI image escaped its output directory"));
            }
            let metadata = tokio::fs::metadata(&path).await?;
            if metadata.len() > MAX_IMAGE_BYTES {
                return Err(anyhow!(
                    "registered CLI image exceeds {MAX_IMAGE_BYTES} bytes"
                ));
            }
            images.push(tool_image_to_data_uri(&tokio::fs::read(path).await?)?);
        }
        Ok(images)
    }
}

#[async_trait]
impl Tool for RegisteredCliTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> Value {
        self.config.parameters.clone().unwrap_or_else(|| {
            serde_json::json!({
                "type":"object",
                "properties": { self.config.input_field.clone(): {
                    "type":"string",
                    "description":"Opaque command passed as one argv value without shell parsing."
                }},
                "required":[self.config.input_field.clone()],
                "additionalProperties":false
            })
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let argument = if self.config.input_mode == "json" {
            serde_json::to_string(&input)?
        } else {
            input
                .get(&self.config.input_field)
                .and_then(Value::as_str)
                .context("registered CLI input must contain a string command")?
                .to_string()
        };
        let output_dir = output_dir(&ctx.session_id, &ctx.message_id);
        tokio::fs::create_dir_all(&output_dir).await?;
        let result = self.run(&argument, &output_dir, ctx).await;
        let _ = tokio::fs::remove_dir_all(&output_dir).await;
        result
    }
}

impl RegisteredCliTool {
    async fn run(
        &self,
        argument: &str,
        output_dir: &Path,
        ctx: &ToolContext,
    ) -> Result<ToolOutput> {
        let mut child = Command::new(&self.config.executable);
        child
            .args(&self.config.args_prefix)
            .arg(argument)
            .current_dir(&ctx.working_dir)
            .env("OPENCODER_TOOL_OUTPUT_DIR", output_dir)
            .kill_on_drop(true);
        let duration = Duration::from_secs(self.config.timeout_seconds.clamp(1, 900));
        let output = match timeout(duration, child.output()).await {
            Ok(result) => result.with_context(|| format!("execute {}", self.config.executable))?,
            Err(_) => {
                return Ok(ToolOutput::err(format!(
                    "registered CLI timed out after {}s",
                    duration.as_secs()
                )))
            }
        };
        let stdout = bounded_utf8(&output.stdout);
        let stderr = bounded_utf8(&output.stderr);
        if !output.status.success() {
            return Ok(ToolOutput::err(format!(
                "registered CLI exited with {}\nstdout: {stdout}\nstderr: {stderr}",
                output.status
            )));
        }
        let value: Value = match serde_json::from_str(&stdout) {
            Ok(value) => value,
            Err(error) => {
                return Ok(ToolOutput::err(format!(
                    "registered CLI returned invalid JSON: {error}\n{stdout}"
                )))
            }
        };
        if semantic_failure(&value) {
            return Ok(ToolOutput::err(stdout));
        }
        match self.images(&value, output_dir).await {
            Ok(images) => Ok(ToolOutput::ok_with_images(stdout, images)),
            Err(error) => Ok(ToolOutput::err(format!("{error:#}"))),
        }
    }
}

fn output_dir(session_id: &str, message_id: &str) -> PathBuf {
    let safe = |value: &str| {
        value
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(32)
            .collect::<String>()
    };
    std::env::temp_dir().join(format!(
        "opencoder-cli-{}-{}-{}",
        safe(session_id),
        safe(message_id),
        ulid::Ulid::new()
    ))
}

fn semantic_failure(value: &Value) -> bool {
    ["success", "ok"]
        .iter()
        .filter_map(|key| value.get(key).and_then(Value::as_bool))
        .any(|ok| !ok)
}

fn bounded_utf8(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_STDIO_BYTES)]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(dir: &Path) -> ToolContext {
        ToolContext {
            session_id: "s".into(),
            message_id: "m".into(),
            agent: "act".into(),
            working_dir: dir.into(),
            max_output: 4096,
            proxy: None,
        }
    }

    #[tokio::test]
    async fn passes_json_as_one_argv_without_shell() {
        let dir = tempfile::tempdir().unwrap();
        let tool = RegisteredCliTool::new(
            CliToolConfig {
                name: "fixture".into(),
                executable: "/usr/bin/printf".into(),
                args_prefix: vec!["%s".into()],
                input_field: "command".into(),
                input_mode: "json".into(),
                parameters: None,
                image_path_pointers: vec![],
                timeout_seconds: 5,
            },
            "fixture".into(),
        )
        .unwrap();
        let output = tool
            .execute(
                serde_json::json!({"command":"$(false); `false`"}),
                &context(dir.path()),
            )
            .await
            .unwrap();
        assert!(!output.is_error);
        assert_eq!(
            serde_json::from_str::<Value>(&output.content).unwrap()["command"],
            "$(false); `false`"
        );
    }

    #[tokio::test]
    async fn rejects_images_outside_private_output_directory() {
        let dir = tempfile::tempdir().unwrap();
        let tool = RegisteredCliTool::new(
            CliToolConfig {
                name: "fixture".into(),
                executable: "/usr/bin/printf".into(),
                args_prefix: vec![
                    "{\"success\":true,\"data\":{\"local_image_path\":\"/etc/hosts\"}}".into(),
                ],
                input_field: "command".into(),
                input_mode: "field".into(),
                parameters: None,
                image_path_pointers: vec!["/data/local_image_path".into()],
                timeout_seconds: 5,
            },
            "fixture".into(),
        )
        .unwrap();
        let output = tool
            .execute(serde_json::json!({"command":""}), &context(dir.path()))
            .await
            .unwrap();
        assert!(output.is_error);
        assert!(output.content.contains("escaped"));
    }
}

//! Pure Codex tool-item projection. Inputs and outputs preserve source payloads.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

pub struct ToolProjection {
    pub name: String,
    pub input: Value,
    pub output: String,
    pub is_error: bool,
    pub images: Vec<String>,
}

pub fn tool(item: &Value) -> Result<ToolProjection> {
    let kind = item["type"].as_str().unwrap_or("");
    let failed = matches!(item["status"].as_str(), Some("failed" | "declined"))
        || item.get("error").is_some_and(|v| !v.is_null());
    let (name, input, output) = match kind {
        "command_execution" => (
            "bash".into(),
            json!({"command":item["command"].as_str().context("Codex command missing")?}),
            item["aggregated_output"]
                .as_str()
                .context("Codex command output missing")?
                .to_owned(),
        ),
        "file_change" => (
            "file_change".into(),
            json!({"changes":item["changes"].as_array().context("Codex file changes missing")?}),
            item.to_string(),
        ),
        "mcp_tool_call" => (
            format!(
                "mcp__{}__{}",
                item["server"]
                    .as_str()
                    .context("Codex MCP server missing")?,
                item["tool"].as_str().context("Codex MCP tool missing")?
            ),
            item.get("arguments").cloned().unwrap_or(Value::Null),
            if failed {
                item["error"].to_string()
            } else {
                item["result"].to_string()
            },
        ),
        "web_search" => (
            "web_search".into(),
            json!({"query":item["query"],"action":item["action"]}),
            item.to_string(),
        ),
        "collab_tool_call" => (
            format!("codex_{}", item["tool"].as_str().unwrap_or("collab")),
            item.clone(),
            item.to_string(),
        ),
        "todo_list" => (
            "update_plan".into(),
            json!({"items":item["items"].as_array().context("Codex TODO items missing")?}),
            item.to_string(),
        ),
        _ => bail!("unsupported Codex item type: {kind}"),
    };
    let images = item["result"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| {
            if v["type"] == "image" {
                Some(format!(
                    "data:{};base64,{}",
                    v["mimeType"].as_str()?,
                    v["data"].as_str()?
                ))
            } else {
                None
            }
        })
        .collect();
    let failed = failed || item["exit_code"].as_i64().is_some_and(|n| n != 0);
    Ok(ToolProjection {
        name,
        input,
        output,
        is_error: failed,
        images,
    })
}

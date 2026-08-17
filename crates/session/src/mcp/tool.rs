//! Wraps a remote MCP tool in the local `Tool` trait so it is transparently
//! dispatched like any builtin tool.
//!
//! Each tool is named `mcp__{server}__{tool}` to avoid collisions with builtin
//! tools and to allow the runner's `ToolFilter` to identify MCP tools by prefix.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{Tool, ToolArc, ToolContext, ToolOutput};
use serde_json::Value;

use super::client::McpClient;
use super::protocol::ToolInfo;

pub struct McpTool {
    full_name: String,
    tool_name: String,
    description: String,
    parameters: Value,
    client: Arc<McpClient>,
}

impl McpTool {
    fn make_full_name(server: &str, tool: &str) -> String {
        format!("mcp__{server}__{tool}")
    }

    // Mirrors `normalized_server_name` in `crates/tui/src/mcp_menu/patch.rs`
    // (the TUI save-time collision guard): `-` and `.` both become `_`, so
    // `a-b` / `a.b` / `a_b` share one tool prefix. Deliberately duplicated
    // (one line each side, cross-referenced) instead of a cross-crate dep.
    fn sanitize_server_name(name: &str) -> String {
        name.replace(['-', '.'], "_")
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.full_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> Value {
        self.parameters.clone()
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let result = self.client.call_tool(&self.tool_name, &input).await?;

        let text = result.text();
        if result.is_error {
            Ok(ToolOutput::err(text))
        } else {
            Ok(ToolOutput::ok(text))
        }
    }
}

/// Build `ToolArc` wrappers for all tools advertised by a single MCP server.
pub fn build_tools(
    client: Arc<McpClient>,
    server_name: &str,
    tools: Vec<ToolInfo>,
) -> Vec<ToolArc> {
    let sanitized = McpTool::sanitize_server_name(server_name);
    tools
        .into_iter()
        .map(|info| {
            let tool_name = info.name.clone();
            let full_name = McpTool::make_full_name(&sanitized, &tool_name);
            Arc::new(McpTool {
                full_name,
                tool_name,
                description: info.description.clone().unwrap_or_else(|| {
                    format!("MCP tool `{}` from server `{}`", info.name, server_name)
                }),
                parameters: info.input_schema,
                client: Arc::clone(&client),
            }) as ToolArc
        })
        .collect()
}

/// The `mcp__` prefix used for all MCP tools.
pub const MCP_PREFIX: &str = "mcp__";

/// True when `name` starts with the MCP tool prefix.
pub fn is_mcp_tool(name: &str) -> bool {
    name.starts_with(MCP_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::transport::{McpTransport, MockTransport};

    fn mock_client() -> (Arc<McpClient>, MockTransport) {
        let (a, b) = MockTransport::pair();
        (McpClient::new(Arc::new(a)), b)
    }

    #[tokio::test]
    async fn mcp_tool_execute_returns_ok_output() {
        let (client, server_side) = mock_client();
        let peer = server_side.clone();
        tokio::spawn(async move {
            let line = peer.recv().await.unwrap();
            let req: serde_json::Value = serde_json::from_str(&line).unwrap();
            let id = req["id"].as_u64().unwrap();
            peer.send_raw(
                serde_json::json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "content": [{"type": "text", "text": "42"}],
                        "isError": false
                    }
                })
                .to_string(),
            );
        });

        let tools = build_tools(
            client,
            "calc",
            vec![ToolInfo {
                name: "add".into(),
                description: Some("add two numbers".into()),
                input_schema: serde_json::json!({"type": "object"}),
            }],
        );
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "mcp__calc__add");

        let ctx = ToolContext {
            session_id: "s".into(),
            message_id: "m".into(),
            agent: "act".into(),
            working_dir: std::path::PathBuf::from("."),
            max_output: 4096,
            proxy: None,
        };
        let out = tools[0]
            .execute(serde_json::json!({"a": 1, "b": 2}), &ctx)
            .await
            .unwrap();
        assert!(!out.is_error);
        assert_eq!(out.content, "42");
    }

    #[tokio::test]
    async fn mcp_tool_execute_propagates_error() {
        let (client, server_side) = mock_client();
        let peer = server_side.clone();
        tokio::spawn(async move {
            let line = peer.recv().await.unwrap();
            let req: serde_json::Value = serde_json::from_str(&line).unwrap();
            let id = req["id"].as_u64().unwrap();
            peer.send_raw(
                serde_json::json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "content": [{"type": "text", "text": "division by zero"}],
                        "isError": true
                    }
                })
                .to_string(),
            );
        });

        let tools = build_tools(
            client,
            "calc",
            vec![ToolInfo {
                name: "div".into(),
                description: None,
                input_schema: serde_json::json!({"type": "object"}),
            }],
        );

        let ctx = ToolContext {
            session_id: "s".into(),
            message_id: "m".into(),
            agent: "act".into(),
            working_dir: std::path::PathBuf::from("."),
            max_output: 4096,
            proxy: None,
        };
        let out = tools[0].execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(out.is_error);
        assert_eq!(out.content, "division by zero");
    }

    #[test]
    fn is_mcp_tool_prefix_check() {
        assert!(is_mcp_tool("mcp__github__create_issue"));
        assert!(is_mcp_tool("mcp__a__b"));
        assert!(!is_mcp_tool("bash"));
        assert!(!is_mcp_tool("task"));
    }

    #[tokio::test]
    async fn build_tools_assigns_full_names() {
        let (client, _) = mock_client();
        let tools = build_tools(
            client,
            "my-server",
            vec![
                ToolInfo {
                    name: "tool1".into(),
                    description: Some("first".into()),
                    input_schema: serde_json::json!({"type": "object"}),
                },
                ToolInfo {
                    name: "tool2".into(),
                    description: None,
                    input_schema: serde_json::json!({"type": "object"}),
                },
            ],
        );
        assert_eq!(tools[0].name(), "mcp__my_server__tool1");
        assert_eq!(tools[1].name(), "mcp__my_server__tool2");
        // Description fallback for tools without one.
        assert!(tools[1].description().contains("my-server"));
    }
}

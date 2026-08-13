//! JSON-RPC 2.0 and MCP protocol types.
//!
//! All MCP field names are `snake_case`-compatible with the wire format
//! (`protocolVersion`, `serverInfo`, etc.) — serde aliases cover both
//! snake_case and the canonical camelCase sent by MCP servers.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---- JSON-RPC 2.0 envelope ----

/// A JSON-RPC request.  `id` is optional: `None` produces a *notification*
/// (fire-and-forget) which needs no response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    #[serde(flatten)]
    pub base: JsonRpcBase,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// Fields common to every JSON-RPC message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcBase {
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
}

/// A JSON-RPC response — the interesting part is `result` (or `error`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JSON-RPC error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for JsonRpcError {}

// ---- MCP protocol types ----

/// Payload sent in `initialize`.
#[derive(Debug, Clone, Serialize)]
pub struct InitializeParams {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    pub capabilities: Value,
    #[serde(rename = "clientInfo")]
    pub client_info: ClientInfo,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

/// Parsed `initialize` result.
#[derive(Debug, Clone, Deserialize)]
pub struct InitializeResult {
    #[serde(alias = "protocolVersion", default)]
    #[allow(dead_code)]
    pub protocol_version: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub capabilities: Value,
    #[serde(alias = "serverInfo", default)]
    #[allow(dead_code)]
    pub server_info: Value,
}

/// A single tool advertised by the server in `tools/list`.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(
        rename = "inputSchema",
        alias = "input_schema",
        default = "default_input_schema"
    )]
    pub input_schema: Value,
}

fn default_input_schema() -> Value {
    serde_json::json!({"type": "object", "properties": {}})
}

/// Result of `tools/list`.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolsListResult {
    #[serde(default)]
    pub tools: Vec<ToolInfo>,
}

/// One content item inside a `tools/call` response.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolContent {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub content_type: String,
    #[serde(default)]
    pub text: Option<String>,
}

/// Result of `tools/call`.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolCallResult {
    #[serde(default)]
    pub content: Vec<ToolContent>,
    #[serde(rename = "isError", default)]
    pub is_error: bool,
}

impl ToolCallResult {
    /// Concatenate all `text` content blocks into a single string.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|c| c.text.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Protocol version we advertise.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serializes_with_method_and_params() {
        let req = JsonRpcRequest {
            base: JsonRpcBase {
                jsonrpc: "2.0".into(),
                id: Some(1),
            },
            method: "tools/list".into(),
            params: Some(serde_json::json!({})),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"method\":\"tools/list\""));
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"id\":1"));
    }

    #[test]
    fn response_with_camel_case_field_deserializes() {
        // Real MCP servers send camelCase keys.
        let raw = r#"{
            "jsonrpc":"2.0","id":1,
            "result":{
                "protocolVersion":"2024-11-05",
                "capabilities":{"tools":{}},
                "serverInfo":{"name":"test","version":"1.0"}
            }
        }"#;
        let resp: JsonRpcResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(resp.id, Some(1));
        let result: InitializeResult =
            serde_json::from_value(resp.result.unwrap()).unwrap();
        assert_eq!(result.protocol_version, "2024-11-05");
        assert!(result.server_info.is_object());
    }

    #[test]
    fn tool_info_with_snake_case_alias() {
        let raw = r#"{
            "name":"echo",
            "description":"echoes input",
            "input_schema":{"type":"object","properties":{"msg":{"type":"string"}}}
        }"#;
        let info: ToolInfo = serde_json::from_str(raw).unwrap();
        assert_eq!(info.name, "echo");
        assert!(info.description.as_deref() == Some("echoes input"));
        assert!(info.input_schema["properties"]["msg"]["type"] == "string");
    }

    #[test]
    fn tool_call_result_text_joins_blocks() {
        let raw = r#"{
            "content":[
                {"type":"text","text":"line1"},
                {"type":"text","text":"line2"}
            ],
            "isError":false
        }"#;
        let result: ToolCallResult = serde_json::from_str(raw).unwrap();
        assert!(!result.is_error);
        assert_eq!(result.text(), "line1\nline2");
    }

    #[test]
    fn tool_call_result_is_error_flag() {
        let raw = r#"{"content":[{"type":"text","text":"oops"}],"isError":true}"#;
        let result: ToolCallResult = serde_json::from_str(raw).unwrap();
        assert!(result.is_error);
        assert_eq!(result.text(), "oops");
    }
}

//! MCP JSON-RPC client.
//!
//! Owns a transport, dispatches incoming responses to waiting callers via
//! `oneshot` channels keyed by JSON-RPC id, and provides high-level helpers
//! for the three MCP methods we need: `initialize`, `tools/list`, `tools/call`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::Value;
use tokio::sync::oneshot;

use super::protocol::{
    ClientInfo, InitializeParams, InitializeResult, JsonRpcBase, JsonRpcRequest, JsonRpcResponse,
    ToolCallResult, ToolsListResult, PROTOCOL_VERSION,
};
use super::transport::McpTransport;

/// Timeout for the `initialize` handshake (server may be slow on first launch).
const INIT_TIMEOUT: Duration = Duration::from_secs(30);
/// Timeout for `tools/list`.
const LIST_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout for individual `tools/call` invocations.
const CALL_TIMEOUT: Duration = Duration::from_secs(300);

pub struct McpClient {
    transport: Arc<dyn McpTransport>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>,
}

impl McpClient {
    /// Create a new client and spawn the background reader task that routes
    /// incoming JSON-RPC responses to waiting callers.
    pub fn new(transport: Arc<dyn McpTransport>) -> Arc<Self> {
        let client = Arc::new(Self {
            transport,
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
        });
        client.spawn_reader();
        client
    }

    fn spawn_reader(self: &Arc<Self>) {
        let client = Arc::clone(self);
        tokio::spawn(async move {
            while let Ok(line) = client.transport.recv().await {
                if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(&line) {
                    let id = match resp.id {
                        Some(id) => id,
                        None => continue, // notification, ignore
                    };
                    if let Some(tx) = client.pending.lock().unwrap().remove(&id) {
                        let _ = tx.send(resp);
                    }
                }
            }
            // Fail all pending requests on transport close.
            client.pending.lock().unwrap().clear();
        });
    }

    /// Send a JSON-RPC *request* and await the `result` field of its response.
    async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);

        let req = JsonRpcRequest {
            base: JsonRpcBase {
                jsonrpc: "2.0".into(),
                id: Some(id),
            },
            method: method.to_string(),
            params: Some(params),
        };
        let msg = serde_json::to_string(&req)?;
        self.transport.send(&msg).await?;

        let resp = tokio::time::timeout(timeout, rx).await.map_err(|_| {
            self.pending.lock().unwrap().remove(&id);
            anyhow!("MCP `{method}` timed out after {:?}", timeout)
        })??;

        if let Some(err) = resp.error {
            return Err(anyhow!(err.to_string()));
        }
        resp.result
            .ok_or_else(|| anyhow!("MCP `{method}` returned no result"))
    }

    /// Send a JSON-RPC *notification* (no id, no response expected).
    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let req = JsonRpcRequest {
            base: JsonRpcBase {
                jsonrpc: "2.0".into(),
                id: None,
            },
            method: method.to_string(),
            params: if params.is_null() { None } else { Some(params) },
        };
        let msg = serde_json::to_string(&req)?;
        self.transport.send(&msg).await?;
        Ok(())
    }

    // ---- High-level MCP methods ----

    /// Perform the MCP initialize handshake.
    pub async fn initialize(&self) -> Result<InitializeResult> {
        let params = serde_json::to_value(InitializeParams {
            protocol_version: PROTOCOL_VERSION.to_string(),
            capabilities: serde_json::json!({}),
            client_info: ClientInfo {
                name: "opencoder".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        })?;
        let result_val = self.request("initialize", params, INIT_TIMEOUT).await?;
        let init_result: InitializeResult = serde_json::from_value(result_val)?;

        // Per spec, the client must send this notification after initialize.
        self.notify("notifications/initialized", Value::Null)
            .await?;

        Ok(init_result)
    }

    /// List available tools from the server.
    pub async fn list_tools(&self) -> Result<Vec<super::protocol::ToolInfo>> {
        let result_val = self
            .request("tools/list", serde_json::json!({}), LIST_TIMEOUT)
            .await?;
        let list: ToolsListResult = serde_json::from_value(result_val)?;
        Ok(list.tools)
    }

    /// Call a tool by name with the given arguments.
    pub async fn call_tool(&self, name: &str, arguments: &Value) -> Result<ToolCallResult> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments,
        });
        let result_val = self.request("tools/call", params, CALL_TIMEOUT).await?;
        let call_result: ToolCallResult = serde_json::from_value(result_val)?;
        Ok(call_result)
    }

    /// Close the underlying transport.
    pub async fn close(&self) {
        self.transport.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::transport::{McpTransport, MockTransport};

    fn mock_client() -> (Arc<McpClient>, MockTransport) {
        let (a, b) = MockTransport::pair();
        (McpClient::new(Arc::new(a)), b)
    }

    /// Helper: spawn a background task that reads one JSON-RPC request from the
    /// server side and replies with `result`.
    fn auto_responder(
        peer: MockTransport,
        result: Value,
    ) -> tokio::task::JoinHandle<(u64, String)> {
        tokio::spawn(async move {
            let line = peer.recv().await.unwrap();
            let req: JsonRpcRequest = serde_json::from_str(&line).unwrap();
            let id = req.base.id.unwrap();
            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result,
            });
            peer.send_raw(resp.to_string());
            (id, req.method)
        })
    }

    #[tokio::test]
    async fn initialize_handshake() {
        let (client, server_side) = mock_client();
        let responder = auto_responder(
            server_side.clone(),
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": {"name": "test", "version": "1.0"}
            }),
        );

        let result = client.initialize().await.unwrap();
        let (_, method) = responder.await.unwrap();
        assert_eq!(method, "initialize");
        assert_eq!(result.protocol_version, "2024-11-05");

        // The notification follows; consume it.
        let notif_line = server_side.recv().await.unwrap();
        let notif: JsonRpcRequest = serde_json::from_str(&notif_line).unwrap();
        assert_eq!(notif.method, "notifications/initialized");
        assert!(notif.base.id.is_none());
    }

    #[tokio::test]
    async fn list_tools_returns_advertised_tools() {
        let (client, server_side) = mock_client();
        let responder = auto_responder(
            server_side,
            serde_json::json!({
                "tools": [
                    {"name": "greet", "description": "say hi",
                     "inputSchema": {"type": "object"}},
                    {"name": "echo", "description": "echo text",
                     "inputSchema": {"type": "object"}}
                ]
            }),
        );

        let tools = client.list_tools().await.unwrap();
        let (_, method) = responder.await.unwrap();
        assert_eq!(method, "tools/list");
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "greet");
        assert_eq!(tools[1].name, "echo");
    }

    #[tokio::test]
    async fn call_tool_returns_content() {
        let (client, server_side) = mock_client();
        let responder = auto_responder(
            server_side,
            serde_json::json!({
                "content": [{"type": "text", "text": "hello from tool"}],
                "isError": false
            }),
        );

        let result = client
            .call_tool("greet", &serde_json::json!({"name": "world"}))
            .await
            .unwrap();
        let (_, method) = responder.await.unwrap();
        assert_eq!(method, "tools/call");
        assert!(!result.is_error);
        assert_eq!(result.text(), "hello from tool");
    }

    #[tokio::test]
    async fn call_tool_propagates_error_response() {
        let (client, server_side) = mock_client();
        let peer = server_side.clone();
        tokio::spawn(async move {
            let line = peer.recv().await.unwrap();
            let req: JsonRpcRequest = serde_json::from_str(&line).unwrap();
            let id = req.base.id.unwrap();
            peer.send_raw(
                serde_json::json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": {"code": -32601, "message": "tool not found"}
                })
                .to_string(),
            );
        });

        let result = client.call_tool("missing", &Value::Null).await;
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err());
        assert!(msg.contains("tool not found"));
    }
}

//! Minimal MCP server for integration testing.
//!
//! Implements the JSON-RPC 2.0 protocol over stdio:
//! - `initialize` → returns server info + capabilities
//! - `notifications/initialized` → ignored (no response)
//! - `tools/list` → returns two test tools (`echo`, `add`)
//! - `tools/call` → dispatches to the tool and returns content
//!
//! Compiled as a separate test binary; invoked by `mcp_integration.rs` tests
//! via `cargo test --test mcp_mock_server` (which just builds it) and then
//! spawned as a child process in `mcp_integration.rs`.
//!
//! The binary name is derived from the file name: `mcp_mock_server`.

use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let req: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let method = req["method"].as_str().unwrap_or("");
        let id = req.get("id").cloned();

        match method {
            "initialize" => {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "mock-mcp", "version": "0.1.0"}
                    }
                });
                writeln!(out, "{}", resp).unwrap();
                out.flush().unwrap();
            }
            "notifications/initialized" => {
                // Notification — no response.
            }
            "tools/list" => {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "tools": [
                            {
                                "name": "echo",
                                "description": "Echo back the input text",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "text": {"type": "string", "description": "Text to echo"}
                                    },
                                    "required": ["text"]
                                }
                            },
                            {
                                "name": "add",
                                "description": "Add two integers",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "a": {"type": "integer", "description": "First number"},
                                        "b": {"type": "integer", "description": "Second number"}
                                    },
                                    "required": ["a", "b"]
                                }
                            }
                        ]
                    }
                });
                writeln!(out, "{}", resp).unwrap();
                out.flush().unwrap();
            }
            "tools/call" => {
                let tool_name = req["params"]["name"].as_str().unwrap_or("");
                let args = &req["params"]["arguments"];
                let (text, is_error) = match tool_name {
                    "echo" => {
                        let t = args["text"].as_str().unwrap_or("(empty)");
                        (format!("echo: {t}"), false)
                    }
                    "add" => {
                        let a = args["a"].as_i64().unwrap_or(0);
                        let b = args["b"].as_i64().unwrap_or(0);
                        (format!("{}", a + b), false)
                    }
                    _ => (format!("unknown tool: {tool_name}"), true),
                };
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{"type": "text", "text": text}],
                        "isError": is_error
                    }
                });
                writeln!(out, "{}", resp).unwrap();
                out.flush().unwrap();
            }
            _ => {
                if let Some(id) = id {
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {"code": -32601, "message": "method not found"}
                    });
                    writeln!(out, "{}", resp).unwrap();
                    out.flush().unwrap();
                }
            }
        }
    }
}

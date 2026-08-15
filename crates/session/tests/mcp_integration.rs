//! Integration tests for the MCP client over a real stdio transport.
//!
//! These tests spawn the `mcp_mock_server` binary (compiled from
//! `bin/mcp_mock_server.rs`) and exercise the full MCP lifecycle:
//! connect → initialize → discover tools → call a tool → disconnect.

use std::collections::HashMap;
use std::path::PathBuf;

use opencoder_core::config::McpServerConfig;

/// Resolve the path to the mock MCP server binary built by cargo.
fn mock_server_path() -> PathBuf {
    // CARGO_BIN_EXE_mcp_mock_server is set when running via `cargo test`.
    PathBuf::from(
        std::env::var("CARGO_BIN_EXE_mcp_mock_server")
            .expect("CARGO_BIN_EXE_mcp_mock_server not set"),
    )
}

/// Build a config pointing at the mock server.
fn mock_server_config() -> McpServerConfig {
    McpServerConfig {
        enabled: true,
        inject_to: opencoder_core::InjectionTarget::Parent,
        command: Some(mock_server_path().to_string_lossy().to_string()),
        args: vec![],
        env: HashMap::new(),
        url: None,
    }
}

fn test_session_id() -> &'static str {
    "mcp-integration-test"
}

#[tokio::test]
async fn sync_connects_and_discovers_tools() {
    let session_id = test_session_id();
    let desired = vec![("mock".to_string(), mock_server_config())];

    opencoder_session::mcp::sync(session_id, &desired).await;

    let tools = opencoder_session::mcp::tools_for(session_id);
    assert!(tools.len() >= 2, "expected at least 2 tools, got {}", tools.len());
    assert!(tools.contains_key("mcp__mock__echo"));
    assert!(tools.contains_key("mcp__mock__add"));

    let status = opencoder_session::mcp::status_for(session_id);
    assert_eq!(status.len(), 1);
    match &status[0].1 {
        opencoder_session::mcp::ConnStatus::Connected { tool_count } => {
            assert_eq!(*tool_count, 2, "expected 2 tools discovered");
        }
        other => panic!("expected Connected, got {other:?}"),
    }

    opencoder_session::mcp::cleanup(session_id).await;
}

#[tokio::test]
async fn call_echo_tool_via_registry() {
    let session_id = "mcp-integration-echo";
    let desired = vec![("mock".to_string(), mock_server_config())];

    opencoder_session::mcp::sync(session_id, &desired).await;
    let tools = opencoder_session::mcp::tools_for(session_id);
    let echo = tools
        .get("mcp__mock__echo")
        .expect("echo tool should be registered");

    let ctx = opencoder_core::ToolContext {
        session_id: session_id.into(),
        message_id: "m1".into(),
        agent: "act".into(),
        working_dir: std::path::PathBuf::from("."),
        max_output: 4096,
        proxy: None,
    };
    let out = echo
        .execute(
            serde_json::json!({"text": "hello world"}),
            &ctx,
        )
        .await
        .expect("echo execute should succeed");
    assert!(!out.is_error);
    assert_eq!(out.content, "echo: hello world");

    opencoder_session::mcp::cleanup(session_id).await;
}

#[tokio::test]
async fn call_add_tool_returns_sum() {
    let session_id = "mcp-integration-add";
    let desired = vec![("mock".to_string(), mock_server_config())];

    opencoder_session::mcp::sync(session_id, &desired).await;
    let tools = opencoder_session::mcp::tools_for(session_id);
    let add = tools
        .get("mcp__mock__add")
        .expect("add tool should be registered");

    let ctx = opencoder_core::ToolContext {
        session_id: session_id.into(),
        message_id: "m1".into(),
        agent: "act".into(),
        working_dir: std::path::PathBuf::from("."),
        max_output: 4096,
        proxy: None,
    };
    let out = add
        .execute(serde_json::json!({"a": 7, "b": 35}), &ctx)
        .await
        .expect("add execute should succeed");
    assert!(!out.is_error);
    assert_eq!(out.content, "42");

    opencoder_session::mcp::cleanup(session_id).await;
}

#[tokio::test]
async fn sync_idempotent_does_not_reconnect() {
    let session_id = "mcp-integration-idempotent";
    let desired = vec![("mock".to_string(), mock_server_config())];

    // First sync connects.
    opencoder_session::mcp::sync(session_id, &desired).await;
    let tools1 = opencoder_session::mcp::tools_for(session_id);

    // Second sync with same desired should not reconnect.
    opencoder_session::mcp::sync(session_id, &desired).await;
    let tools2 = opencoder_session::mcp::tools_for(session_id);

    assert_eq!(tools1.len(), tools2.len());

    opencoder_session::mcp::cleanup(session_id).await;
}

#[tokio::test]
async fn disable_server_removes_tools() {
    let session_id = "mcp-integration-disable";
    let desired = vec![("mock".to_string(), mock_server_config())];

    opencoder_session::mcp::sync(session_id, &desired).await;
    assert!(!opencoder_session::mcp::tools_for(session_id).is_empty());

    // Sync with empty desired — should remove the server.
    opencoder_session::mcp::sync(session_id, &[]).await;
    assert!(opencoder_session::mcp::tools_for(session_id).is_empty());
    assert!(opencoder_session::mcp::status_for(session_id).is_empty());

    opencoder_session::mcp::cleanup(session_id).await;
}

#[tokio::test]
async fn bad_command_records_failed_status() {
    let session_id = "mcp-integration-bad";
    let desired = vec![(
        "broken".to_string(),
        McpServerConfig {
            enabled: true,
            command: Some("/nonexistent/binary/path/xyz".into()),
            ..Default::default()
        },
    )];

    opencoder_session::mcp::sync(session_id, &desired).await;

    let status = opencoder_session::mcp::status_for(session_id);
    assert_eq!(status.len(), 1);
    assert!(matches!(
        status[0].1,
        opencoder_session::mcp::ConnStatus::Failed(_)
    ));
    assert!(opencoder_session::mcp::tools_for(session_id).is_empty());

    opencoder_session::mcp::cleanup(session_id).await;
}

#[tokio::test]
async fn cleanup_disconnects_and_clears_pool() {
    let session_id = "mcp-integration-cleanup";
    let desired = vec![("mock".to_string(), mock_server_config())];

    opencoder_session::mcp::sync(session_id, &desired).await;
    assert!(!opencoder_session::mcp::tools_for(session_id).is_empty());

    opencoder_session::mcp::cleanup(session_id).await;
    assert!(opencoder_session::mcp::tools_for(session_id).is_empty());
    assert!(opencoder_session::mcp::status_for(session_id).is_empty());
}

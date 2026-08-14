//! Timeout-routing tests extracted from `execute.rs` to keep the implementation
//! within the repository's per-file size gate.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use opencoder_core::{resolve_agent, Config, Tool, ToolArc, ToolContext, ToolOutput};
use opencoder_llm::{tool_call::CompletedToolCall, ChatStream, MockChatClient};
use serde_json::json;

use super::super::event::Sink;
use super::{execute_call, leaf_tool_timeout, DEFAULT_TOOL_TIMEOUT};
use crate::{SessionEvent, SessionState};

struct HangingTool;

#[async_trait]
impl Tool for HangingTool {
    fn name(&self) -> &str {
        "hang"
    }

    fn description(&self) -> &str {
        "never resolves"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({})
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        std::future::pending::<()>().await;
        unreachable!()
    }
}

fn make_session() -> SessionState {
    SessionState::new(
        "sess-timeout-routing-test",
        resolve_agent("act").unwrap(),
        Config::default(),
        Arc::new(MockChatClient::new()) as Arc<dyn ChatStream>,
        std::env::temp_dir().join("opencer-execute-timeout-tests"),
    )
}

#[test]
fn leaf_tool_timeout_routes_read_edit_search_to_bash_budget() {
    let expected = Some(Duration::from_secs(crate::tools::bash::BASH_TIMEOUT_SECS));
    assert_eq!(leaf_tool_timeout("read"), expected);
    assert_eq!(leaf_tool_timeout("edit"), expected);
    assert_eq!(leaf_tool_timeout("search"), expected);
}

#[test]
fn leaf_tool_timeout_exempts_bash() {
    assert_eq!(leaf_tool_timeout("bash"), None);
}

#[test]
fn leaf_tool_timeout_exempts_question() {
    // Waiting for a human answer has no wall-clock budget; only cancel ends it.
    assert_eq!(leaf_tool_timeout("question"), None);
}

#[test]
fn leaf_tool_timeout_defaults_unknown_tools() {
    assert_eq!(leaf_tool_timeout("ls"), Some(DEFAULT_TOOL_TIMEOUT));
    assert_eq!(
        leaf_tool_timeout("not_a_real_tool"),
        Some(DEFAULT_TOOL_TIMEOUT)
    );
}

/// A hanging tool registered as `read` must use the short routed timeout, not
/// the generic 600-second safety net.
#[tokio::test]
async fn read_tool_times_out_via_execute_call_routing() {
    let session = make_session();
    let registry: HashMap<String, ToolArc> =
        [("read".to_string(), Arc::new(HangingTool) as ToolArc)]
            .into_iter()
            .collect();
    let mut noop: Box<dyn FnMut(SessionEvent) + Send> = Box::new(|_| {});
    let sink: Sink<'_> = Arc::new(Mutex::new(&mut *noop));
    let tc = CompletedToolCall {
        id: "tc-read-route".into(),
        name: "read".into(),
        input: json!({}),
    };

    let out = tokio::time::timeout(Duration::from_secs(5), async {
        execute_call(&tc, &session, &registry, &sink).await
    })
    .await
    .expect("read tool should trip the routed bash budget, not the 600s net");
    assert!(out.is_error);
    assert!(
        out.content.contains("timed out"),
        "expected timeout message, got: {}",
        out.content
    );
}

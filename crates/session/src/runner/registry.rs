//! Tool-registry construction for a session turn, extracted from
//! `runner/mod.rs` to respect the per-file line cap.

use std::collections::HashMap;
use std::sync::Arc;

use opencoder_core::ToolArc;

use crate::{mcp, SessionState};

/// Build the builtin tool registry merged with any MCP tools discovered for
/// this session.  Also synchronises the MCP connection pool so that enabled
/// servers are connected (and disabled ones disconnected) before the turn.
pub(super) async fn build_full_registry(session: &SessionState) -> HashMap<String, ToolArc> {
    let desired: Vec<(String, opencoder_core::config::McpServerConfig)> = session
        .config
        .enabled_mcp_servers()
        .into_iter()
        .map(|(n, c)| (n, c.clone()))
        .collect();
    if !desired.is_empty() {
        mcp::pool::sync(&session.id, &desired).await;
    }
    let mut reg = crate::tools::registry();
    // The registry's `question` entry carries a placeholder hub (schema/token
    // estimation only). Rebind it to this session's shared hub so an attached
    // frontend resolves tool results mid-turn.
    reg.insert(
        "question".to_string(),
        Arc::new(crate::tools::question::QuestionTool::new(
            session.question_hub.clone(),
        )),
    );
    if mcp::pool::has_mcp_tools(&session.id) {
        reg.extend(mcp::pool::tools_for(&session.id));
    }
    reg
}

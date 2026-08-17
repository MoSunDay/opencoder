//! Process-global MCP connection pool.
//!
//! Mirrors the "static map keyed by session_id" pattern used elsewhere in the
//! codebase.  Connections live outside `SessionState` so they survive the
//! per-drain rebuild of session state in web mode.
//!
//! The pool uses a `std::sync::Mutex` held only for short, synchronous map
//! operations.  All async work (spawn, initialize, list_tools) happens *before*
//! the guard is acquired.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

use opencoder_core::config::McpServerConfig;
use opencoder_core::ToolArc;

use super::client::McpClient;
use super::tool::{build_tools, is_mcp_tool};
use super::transport;

/// Status of a single server connection, for system-prompt injection.
#[derive(Debug, Clone)]
pub enum ConnStatus {
    Connected { tool_count: usize },
    Failed(String),
}

struct McpConnection {
    #[allow(dead_code)]
    client: Arc<McpClient>,
    tools: Vec<ToolArc>,
    status: ConnStatus,
}

#[derive(Default)]
struct McpSession {
    connections: HashMap<String, McpConnection>,
}

/// Global pool: session_id → per-session connection map.
static MCP_POOL: LazyLock<Mutex<HashMap<String, McpSession>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Synchronise the pool for `session_id` against the desired server list.
///
/// - Removes connections no longer in `desired`.
/// - Connects servers that appear in `desired` but are not yet connected.
/// - Already-connected servers are left untouched (idempotent).
/// - A server that fails to connect is recorded with `ConnStatus::Failed` so
///   it appears in the system prompt; it does **not** block other servers.
pub async fn sync(session_id: &str, desired: &[(String, McpServerConfig)]) {
    // Phase 1: Determine adds and removes under a brief lock.
    let (to_remove, to_add): (Vec<String>, Vec<(String, McpServerConfig)>) = {
        let pool = MCP_POOL.lock().unwrap();
        let current: HashSet<String> = pool
            .get(session_id)
            .map(|s| s.connections.keys().cloned().collect())
            .unwrap_or_default();
        let desired_set: HashSet<String> = desired.iter().map(|(n, _)| n.clone()).collect();

        let remove = current.difference(&desired_set).cloned().collect();
        let add = desired
            .iter()
            .filter(|(n, _)| !current.contains(n))
            .cloned()
            .collect();
        (remove, add)
    };

    // Phase 2: Connect new servers (async, outside the lock).
    let mut new_conns: Vec<(String, McpConnection)> = Vec::new();
    for (name, cfg) in to_add {
        match connect_server(&name, &cfg).await {
            Ok(conn) => new_conns.push((name, conn)),
            Err(e) => {
                tracing::warn!(server = %name, error = %e, "MCP server connection failed");
                new_conns.push((
                    name,
                    McpConnection {
                        client: McpClient::new(Arc::new(transport::MockTransport::pair().0)),
                        tools: vec![],
                        status: ConnStatus::Failed(format!("{e:#}")),
                    },
                ));
            }
        }
    }

    // Phase 3: Apply changes (brief lock).
    let mut pool = MCP_POOL.lock().unwrap();
    let session = pool.entry(session_id.to_string()).or_default();
    for name in to_remove {
        if let Some(conn) = session.connections.remove(&name) {
            // Dropping the connection closes the transport (kill_on_drop).
            drop(conn);
        }
    }
    for (name, conn) in new_conns {
        session.connections.insert(name, conn);
    }
}

async fn connect_server(name: &str, cfg: &McpServerConfig) -> anyhow::Result<McpConnection> {
    let transport = transport::build_from_config(cfg)?;
    let transport_arc = Arc::from(transport);
    let client = McpClient::new(transport_arc);
    client.initialize().await?;
    let tools_info = client.list_tools().await?;
    let tool_count = tools_info.len();
    let tools = build_tools(client.clone(), name, tools_info);
    Ok(McpConnection {
        client,
        tools,
        status: ConnStatus::Connected { tool_count },
    })
}

/// Merge per-server tool lists into one `full_name → ToolArc` map.
///
/// Bug #14: server names are sanitized to a shared prefix when tool names
/// are built (`a-b` / `a.b` / `a_b` → `mcp__a_b__…`), so two distinct
/// servers can advertise colliding full names. The first registrant wins;
/// later duplicates are dropped with a `warn!` instead of silently
/// overwriting (which cross-wired tools and bypassed `inject_to` scoping).
/// `kept_by` tracks which server contributed each surviving name so the log
/// names both sides. Input order decides "first" — callers sort by server
/// name to keep that deterministic.
fn merge_tools(per_server: Vec<(String, Vec<ToolArc>)>) -> HashMap<String, ToolArc> {
    let mut out: HashMap<String, ToolArc> = HashMap::new();
    let mut kept_by: HashMap<String, String> = HashMap::new();
    for (server, tools) in per_server {
        for tool in tools {
            let name = tool.name().to_string();
            match kept_by.get(&name) {
                Some(owner) => tracing::warn!(
                    tool = %name,
                    server = server.as_str(),
                    kept_by = owner.as_str(),
                    "MCP tool name collision after server-name normalization; dropping duplicate"
                ),
                None => {
                    // Clone (not move): `server` may still be needed by a
                    // later tool of the same server hitting the warn above.
                    kept_by.insert(name.clone(), server.clone());
                    out.insert(name, tool);
                }
            }
        }
    }
    out
}

/// Return all MCP tool wrappers for `session_id` as a `name → ToolArc` map.
pub fn tools_for(session_id: &str) -> HashMap<String, ToolArc> {
    // Snapshot under the lock, merge outside it.
    let per_server: Vec<(String, Vec<ToolArc>)> = {
        let pool = MCP_POOL.lock().unwrap();
        let Some(session) = pool.get(session_id) else {
            return HashMap::new();
        };
        let mut servers: Vec<(String, Vec<ToolArc>)> = session
            .connections
            .iter()
            .filter(|(_, c)| matches!(c.status, ConnStatus::Connected { .. }))
            .map(|(name, c)| {
                (
                    name.clone(),
                    c.tools.iter().map(Arc::clone).collect::<Vec<_>>(),
                )
            })
            .collect();
        // HashMap iteration order is process-randomized; sort by server name
        // so "first registrant wins" in `merge_tools` is stable across runs.
        servers.sort_by(|a, b| a.0.cmp(&b.0));
        servers
    };
    merge_tools(per_server)
}

/// Return per-server status for system-prompt injection.
pub fn status_for(session_id: &str) -> Vec<(String, ConnStatus)> {
    let pool = MCP_POOL.lock().unwrap();
    let Some(session) = pool.get(session_id) else {
        return Vec::new();
    };
    let mut out: Vec<(String, ConnStatus)> = session
        .connections
        .iter()
        .map(|(n, c)| (n.clone(), c.status.clone()))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Remove and drop all connections for `session_id`.
pub async fn cleanup(session_id: &str) {
    let removed = {
        let mut pool = MCP_POOL.lock().unwrap();
        pool.remove(session_id)
    };
    // Dropping McpConnection closes the transport (kill_on_drop on the child).
    drop(removed);
}

/// Returns `true` if any MCP tool is currently registered for `session_id`.
/// Convenience for callers that want to short-circuit.
pub fn has_mcp_tools(session_id: &str) -> bool {
    let pool = MCP_POOL.lock().unwrap();
    pool.get(session_id)
        .map(|s| {
            s.connections
                .values()
                .any(|c| matches!(c.status, ConnStatus::Connected { .. }))
        })
        .unwrap_or(false)
}

/// Re-export for callers that need to check the prefix.
pub fn is_mcp_tool_name(name: &str) -> bool {
    is_mcp_tool(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::protocol::ToolInfo;
    use transport::MockTransport;

    /// Build real `ToolArc`s for one fake server (no I/O — `build_tools`
    /// only wraps; the mock client's peer may be dropped unused).
    fn server_tools(server: &str, tool: &str, desc: &str) -> Vec<ToolArc> {
        let (client, _peer) = MockTransport::pair();
        build_tools(
            McpClient::new(Arc::new(client)),
            server,
            vec![ToolInfo {
                name: tool.into(),
                description: Some(desc.into()),
                input_schema: serde_json::json!({"type": "object"}),
            }],
        )
    }

    /// Bug #14: `a-b` and `a_b` normalize to the same `mcp__a_b__` prefix,
    /// so their same-named tools collide. `merge_tools` must keep exactly
    /// one — the first registrant in input order — instead of silently
    /// overwriting, and must not panic.
    // `McpClient::new` spawns a reader task, so a reactor must exist.
    #[tokio::test]
    async fn merge_tools_drops_duplicate_after_normalization() {
        let merged = merge_tools(vec![
            ("a-b".to_string(), server_tools("a-b", "echo", "from a-b")),
            ("a_b".to_string(), server_tools("a_b", "echo", "from a_b")),
        ]);
        assert_eq!(
            merged.len(),
            1,
            "exactly one survivor, got {:?}",
            merged.keys().collect::<Vec<_>>()
        );
        let tool = merged.get("mcp__a_b__echo").expect("normalized full name");
        assert_eq!(tool.description(), "from a-b", "first registrant wins");
    }

    /// Same inputs, reversed order: the other server's tool survives —
    /// proving first-wins follows input order (callers sort by server name),
    /// not tool identity.
    // `McpClient::new` spawns a reader task, so a reactor must exist.
    #[tokio::test]
    async fn merge_tools_first_registrant_follows_input_order() {
        let merged = merge_tools(vec![
            ("a_b".to_string(), server_tools("a_b", "echo", "from a_b")),
            ("a-b".to_string(), server_tools("a-b", "echo", "from a-b")),
        ]);
        assert_eq!(merged.len(), 1);
        assert_eq!(
            merged["mcp__a_b__echo"].description(),
            "from a_b",
            "first registrant wins"
        );
    }

    /// Disjoint tool names across servers all survive — the guard only
    /// drops true full-name duplicates.
    // `McpClient::new` spawns a reader task, so a reactor must exist.
    #[tokio::test]
    async fn merge_tools_keeps_disjoint_names() {
        let merged = merge_tools(vec![
            ("a-b".to_string(), server_tools("a-b", "echo", "from a-b")),
            ("c".to_string(), server_tools("c", "echo", "from c")),
        ]);
        assert_eq!(merged.len(), 2);
        assert!(merged.contains_key("mcp__a_b__echo"));
        assert!(merged.contains_key("mcp__c__echo"));
    }

    #[test]
    fn tools_for_empty_session_returns_empty() {
        let tools = tools_for("nonexistent-session");
        assert!(tools.is_empty());
    }

    #[test]
    fn status_for_empty_session_returns_empty() {
        let status = status_for("nonexistent-session");
        assert!(status.is_empty());
    }

    #[tokio::test]
    async fn cleanup_removes_session() {
        // Insert a dummy empty session entry.
        {
            let mut pool = MCP_POOL.lock().unwrap();
            pool.entry("cleanup-test".to_string()).or_default();
        }
        assert!(MCP_POOL.lock().unwrap().contains_key("cleanup-test"));
        cleanup("cleanup-test").await;
        assert!(!MCP_POOL.lock().unwrap().contains_key("cleanup-test"));
    }

    #[tokio::test]
    async fn sync_with_empty_desired_is_noop() {
        // Ensure no panic and empty result.
        sync("noop-session", &[]).await;
        assert!(tools_for("noop-session").is_empty());
        cleanup("noop-session").await;
    }

    #[tokio::test]
    async fn sync_with_bad_command_records_failed_status() {
        let desired = vec![(
            "bad-server".to_string(),
            McpServerConfig {
                enabled: true,
                command: Some("/nonexistent/binary/that/does/not/exist".into()),
                ..Default::default()
            },
        )];
        sync("bad-session", &desired).await;
        let status = status_for("bad-session");
        assert_eq!(status.len(), 1);
        assert!(matches!(status[0].1, ConnStatus::Failed(_)));
        // No tools available.
        assert!(tools_for("bad-session").is_empty());
        cleanup("bad-session").await;
    }
}

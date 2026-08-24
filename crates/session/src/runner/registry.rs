//! Tool-registry construction for a session turn, extracted from
//! `runner/mod.rs` to respect the per-file line cap.

use std::{collections::HashMap, sync::Arc};

use anyhow::{bail, Context, Result};
use opencoder_core::{Tool, ToolArc};

use crate::{mcp, tools, SessionState};

/// Build the builtin tool registry merged with any MCP tools discovered for
/// this session.  Also synchronises the MCP connection pool so that enabled
/// servers are connected (and disabled ones disconnected) before the turn.
pub(super) async fn build_full_registry(
    session: &SessionState,
) -> Result<HashMap<String, ToolArc>> {
    let desired: Vec<(String, opencoder_core::config::McpServerConfig)> = session
        .config
        .enabled_mcp_servers()
        .into_iter()
        .map(|(n, c)| (n, c.clone()))
        .collect();
    if !desired.is_empty() {
        mcp::pool::sync(&session.id, &desired).await;
    }
    let mut registry = tools::registry();
    // The registry's `question` entry carries a placeholder hub (schema/token
    // estimation only). Rebind it to this session's shared hub so an attached
    // frontend resolves tool results mid-turn.
    registry.insert(
        "question".to_string(),
        Arc::new(tools::question::QuestionTool::new(
            session.question_hub.clone(),
        )),
    );
    if mcp::pool::has_mcp_tools(&session.id) {
        registry.extend(mcp::pool::tools_for(&session.id));
    }
    if session.agent.name == "workflow" {
        return Ok(registry);
    }

    for (registration, cli) in session
        .config
        .enabled_cli_for(&session.agent.name, session.agent.mode)
    {
        if let Some(error) = &cli.tool_config_error {
            bail!("invalid registered CLI tool {registration}: {error}");
        }
        let Some(config) = &cli.tool else {
            continue;
        };
        let tool =
            tools::registered_cli::RegisteredCliTool::new(config.clone(), cli.content.clone())
                .with_context(|| format!("invalid registered CLI tool {registration}"))?;
        if registry.contains_key(tool.name()) {
            bail!(
                "registered CLI tool {registration} collides with tool {}",
                tool.name()
            );
        }
        registry.insert(tool.name().into(), Arc::new(tool));
    }
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::{resolve_agent, CliConfig, CliToolConfig, InjectionTarget};
    use opencoder_llm::MockChatClient;

    fn session(agent: &str) -> SessionState {
        let mut config = opencoder_core::Config::default();
        config.cli.insert(
            "fixture".into(),
            CliConfig {
                enabled: true,
                inject_to: InjectionTarget::parent_only(),
                content: "fixture registered CLI".into(),
                tool: Some(CliToolConfig {
                    name: "cli__fixture".into(),
                    executable: "/usr/bin/printf".into(),
                    args_prefix: vec!["%s".into()],
                    input_field: "command".into(),
                    input_mode: "json".into(),
                    parameters: None,
                    image_path_pointers: vec![],
                    timeout_seconds: 5,
                }),
                tool_config_error: None,
            },
        );
        SessionState::new(
            "registry-test",
            resolve_agent(agent).unwrap(),
            config,
            Arc::new(MockChatClient::new()),
            std::env::temp_dir(),
        )
    }

    #[tokio::test]
    async fn registered_cli_is_available_to_primary_agent() {
        let registry = build_full_registry(&session("act")).await.unwrap();
        assert!(registry.contains_key("cli__fixture"));
    }

    #[tokio::test]
    async fn workflow_agent_never_receives_registered_cli() {
        let registry = build_full_registry(&session("workflow")).await.unwrap();
        assert!(!registry.contains_key("cli__fixture"));
    }
}

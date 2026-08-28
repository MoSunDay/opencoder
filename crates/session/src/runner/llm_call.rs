use std::collections::HashMap;

use anyhow::{anyhow, Result};
use opencoder_core::{AgentMode, Config, MessageUsage, ToolArc};
use opencoder_llm::tool_call::CompletedToolCall;
use opencoder_llm::{lower_messages, ChatRequest, ChatStream, LlmEvent, Usage};

use crate::prompt::build_system;
use crate::tools::schema_for;
use crate::SessionState;

use super::event::SessionEvent;
use super::steer::{await_cancel, await_turn_cancel};

pub(super) async fn run_one_llm_call(
    session: &SessionState,
    registry: &HashMap<String, ToolArc>,
    on_event: &mut (impl FnMut(SessionEvent) + Send + ?Sized),
) -> Result<(String, String, Vec<CompletedToolCall>, Option<Usage>)> {
    let mcp_status = mcp_status_for_agent(session, registry);
    let mcp = crate::prompt::mcp_section(&mcp_status);
    let cli = (session.agent.name != "workflow")
        .then(|| {
            crate::prompt::cli_section(
                &session
                    .config
                    .enabled_cli_for(&session.agent.name, session.agent.mode),
            )
        })
        .flatten();
    let runtime = crate::prompt::runtime_sections(mcp.as_deref(), cli.as_deref());
    let system = build_system(&session.agent, &session.working_dir, runtime.as_deref());
    let mut to_send = vec![system];
    to_send.extend(session.messages.iter().cloned());
    // Transient skill-context reminder: derived per call, appended LAST and
    // never persisted, so activating a skill never mutates the payload's
    // persisted prefix. The system prompt itself is rebuilt on every call
    // and re-reads AGENTS.md from disk, so it is NOT byte-stable across
    // calls when AGENTS.md changes on disk.
    if let Some(tail) = crate::skill_context::tail_reminder(session) {
        to_send.push(tail);
    }
    let openai_msgs = lower_messages(&to_send);

    let skill_body = session.skill_prompt_cloned();
    let unlocked = crate::tools::latent::unlocked_from_body(skill_body.as_deref());
    let allowed: HashMap<String, ToolArc> = registry
        .iter()
        .filter(|(name, _)| {
            if crate::mcp::is_mcp_tool(name.as_str()) {
                return session.agent.name != "workflow"
                    && mcp_tool_allowed(
                        &session.config,
                        &session.agent.name,
                        session.agent.mode,
                        name,
                    );
            }
            // Sandbox always sees `question` (base-prompt clarification
            // protocol); other agents need the task-plan/review skill unlock.
            crate::tools::latent::is_visible(name.as_str(), &session.agent, &unlocked)
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let tool_schemas = schema_for(&allowed, session.agent.kind);

    let req = ChatRequest {
        model: session.model.clone(),
        messages: openai_msgs,
        tools: tool_schemas,
        tool_choice: if allowed.is_empty() {
            None
        } else {
            Some("auto".into())
        },
        temperature: None,
        max_tokens: session.config.max_tokens,
        reasoning_effort: session.config.reasoning_effort.clone(),
        cache_salt: crate::cache_salt_for(session),
    };
    let mut rx = session.client.chat_stream(req)?;
    let mut completed: Option<(String, Vec<CompletedToolCall>, Option<Usage>)> = None;
    let mut reasoning_buf = String::new();
    // True once a `Retrying` status has been shown; cleared (with an empty
    // Status event) the moment real content streams so the "↻ retry" badge
    // doesn't linger after recovery.
    let mut retried = false;
    let mut cancel_fut = std::pin::pin!(await_cancel(session));
    let mut turn_cancel_fut = std::pin::pin!(await_turn_cancel(session));
    loop {
        // The event-level idle watchdog now lives inside the streaming client
        // (`ChatClient::run_stream`), which retries stalls before they ever
        // reach this loop. Only session-semantic interrupts remain here: a
        // double-Esc / web interrupt (cancel) and a subagent submit-now steer
        // (turn_cancel).
        tokio::select! {
            biased;
            _ = &mut cancel_fut => {
                on_event(SessionEvent::Status("interrupted".into()));
                return Ok((String::new(), String::new(), Vec::new(), None));
            }
            _ = &mut turn_cancel_fut => {
                // Turn interrupted by subagent steer "submit-now": return an
                // empty turn. The caller (run_loop) detects this via
                // is_turn_cancelled and continues the loop to absorb pending
                // steers — it does NOT break like a real cancel.
                return Ok((String::new(), String::new(), Vec::new(), None));
            }
            ev = rx.recv() => {
                let ev = match ev { Some(ev) => ev, None => break };
                match ev {
                    LlmEvent::TextDelta(t) => {
                        if retried {
                            retried = false;
                            on_event(SessionEvent::Status(String::new()));
                        }
                        on_event(SessionEvent::TextDelta(t));
                    }
                    LlmEvent::ReasoningDelta(r) => {
                        if retried {
                            retried = false;
                            on_event(SessionEvent::Status(String::new()));
                        }
                        reasoning_buf.push_str(&r);
                        on_event(SessionEvent::ReasoningDelta(r));
                    }
                    LlmEvent::ToolCallStart { .. } | LlmEvent::ToolCallDelta { .. } => {}
                    LlmEvent::Completed { text, tool_calls, usage } => {
                        if retried {
                            retried = false;
                            on_event(SessionEvent::Status(String::new()));
                        }
                        completed = Some((text, tool_calls, usage));
                    }
                    LlmEvent::Retrying { attempt, max } => {
                        // Mid-stream retry: the client discarded its partial
                        // response and is regenerating from scratch. Drop any
                        // reasoning deltas accumulated this attempt so they
                        // aren't stitched onto the fresh frame.
                        reasoning_buf.clear();
                        retried = true;
                        on_event(SessionEvent::Status(format!(
                            "\u{21bb} retry {attempt}/{max}"
                        )));
                    }
                    LlmEvent::Error(e) => return Err(anyhow!(e)),
                }
            }
        }
    }
    let (text, tool_calls, usage) =
        completed.ok_or_else(|| anyhow!("stream ended without completion"))?;
    Ok((text, reasoning_buf, tool_calls, usage))
}

fn mcp_tool_allowed(config: &Config, agent: &str, mode: AgentMode, tool_name: &str) -> bool {
    config
        .enabled_mcp_servers_for(agent, mode)
        .into_iter()
        .any(|(name, _)| {
            let prefix = format!("mcp__{}__", name.replace(['-', '.'], "_"));
            tool_name.starts_with(&prefix)
        })
}

fn mcp_status_for_agent(
    session: &SessionState,
    registry: &HashMap<String, ToolArc>,
) -> Vec<(String, crate::mcp::ConnStatus)> {
    let applicable = session
        .config
        .enabled_mcp_servers_for(&session.agent.name, session.agent.mode);
    let live: HashMap<_, _> = crate::mcp::pool::status_for(&session.id)
        .into_iter()
        .collect();
    applicable
        .into_iter()
        .map(|(name, _)| {
            let status = live.get(&name).cloned().unwrap_or_else(|| {
                let prefix = format!("mcp__{}__", name.replace(['-', '.'], "_"));
                let tool_count = registry
                    .keys()
                    .filter(|tool| tool.starts_with(&prefix))
                    .count();
                crate::mcp::ConnStatus::Connected { tool_count }
            });
            (name, status)
        })
        .collect()
}

pub(super) fn core_usage(u: &Usage) -> MessageUsage {
    MessageUsage {
        input_tokens: u.input_tokens,
        output_tokens: u.output_tokens,
        total_tokens: u.total_tokens,
        cache_read_tokens: u.cache_read_tokens,
        cache_creation_tokens: u.cache_creation_tokens,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use anyhow::Result;
    use tokio::sync::mpsc;

    use opencoder_core::{resolve_agent, Config, ToolArc};
    use opencoder_llm::{ChatRequest, ChatStream, LlmEvent, MockChatClient};

    use crate::SessionState;

    use super::run_one_llm_call;

    fn make_session(client: Arc<dyn ChatStream>) -> SessionState {
        let cfg = Config::default();
        SessionState::new(
            "test",
            resolve_agent("act").unwrap(),
            cfg,
            client,
            std::env::temp_dir().join("opencoder-idle-test"),
        )
    }

    #[tokio::test]
    async fn busy_stream_completes() {
        let mock = MockChatClient::new().push_script(vec![
            LlmEvent::TextDelta("hello".into()),
            LlmEvent::Completed {
                text: "hello".into(),
                tool_calls: vec![],
                usage: None,
            },
        ]);
        let session = make_session(Arc::new(mock) as Arc<dyn ChatStream>);
        let registry: HashMap<String, ToolArc> = HashMap::new();
        let result = run_one_llm_call(&session, &registry, &mut |_| {}).await;
        assert!(result.is_ok(), "expected success, got: {:?}", result);
        let (text, _, _, _) = result.unwrap();
        assert_eq!(text, "hello");
    }

    /// A custom `ChatStream` double that emits a fixed event script. Lets us
    /// drive the consumer loop with a synthetic mid-stream `Retrying` event,
    /// which the real `ChatClient` produces when it restarts an interrupted
    /// stream. The mock client cannot express this (it has no notion of
    /// retries), so a dedicated double is needed.
    struct ScriptedClient {
        events: std::sync::Mutex<Option<Vec<LlmEvent>>>,
    }

    impl ChatStream for ScriptedClient {
        fn chat_stream(&self, _req: ChatRequest) -> Result<mpsc::Receiver<LlmEvent>> {
            let events = self
                .events
                .lock()
                .unwrap()
                .take()
                .expect("ScriptedClient used more than once");
            let (tx, rx) = mpsc::channel::<LlmEvent>(128);
            tokio::spawn(async move {
                for ev in events {
                    tokio::task::yield_now().await;
                    if tx.send(ev).await.is_err() {
                        break;
                    }
                }
            });
            Ok(rx)
        }
        fn backend(&self) -> &'static str {
            "scripted"
        }
    }

    /// A mid-stream `Retrying` must discard deltas accumulated so far. The
    /// double streams some partial text + reasoning, signals a retry, then
    /// streams a fresh frame ending in `Completed`. The returned text must be
    /// the FINAL frame's text (never stitched), and reasoning from the dead
    /// first attempt must be cleared.
    #[tokio::test]
    async fn mid_stream_retry_clears_accumulated_state() {
        let double = ScriptedClient {
            events: std::sync::Mutex::new(Some(vec![
                LlmEvent::TextDelta("partial ".into()),
                LlmEvent::ReasoningDelta("dead-thought".into()),
                LlmEvent::Retrying { attempt: 1, max: 3 },
                LlmEvent::TextDelta("final".into()),
                LlmEvent::ReasoningDelta("live-thought".into()),
                LlmEvent::Completed {
                    text: "final".into(),
                    tool_calls: vec![],
                    usage: None,
                },
            ])),
        };
        let session = make_session(Arc::new(double) as Arc<dyn ChatStream>);
        let registry: HashMap<String, ToolArc> = HashMap::new();
        let mut saw_retry = false;
        let result = run_one_llm_call(&session, &registry, &mut |ev| {
            if let crate::runner::SessionEvent::Status(s) = &ev {
                if s.contains("retry") {
                    saw_retry = true;
                }
            }
        })
        .await;
        assert!(result.is_ok(), "expected success, got: {:?}", result);
        let (text, reasoning, _, _) = result.unwrap();
        // Text always comes from the final Completed frame — never stitched
        // across the two attempts.
        assert_eq!(text, "final");
        // Reasoning from the dead first attempt is cleared on retry; only the
        // live frame's reasoning survives.
        assert_eq!(reasoning, "live-thought");
        assert!(saw_retry, "consumer should surface the retry status");
    }

    // ---- MCP ToolFilter tests ----

    /// A minimal fake MCP tool for testing the filter behaviour.
    struct FakeMcpTool {
        tool_name: String,
    }

    #[async_trait::async_trait]
    impl opencoder_core::Tool for FakeMcpTool {
        fn name(&self) -> &str {
            &self.tool_name
        }
        fn description(&self) -> &str {
            "fake MCP tool for testing"
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &opencoder_core::ToolContext,
        ) -> anyhow::Result<opencoder_core::ToolOutput> {
            Ok(opencoder_core::ToolOutput::ok("ok"))
        }
    }

    fn registry_with_mcp() -> HashMap<String, ToolArc> {
        let mut reg = crate::tools::registry();
        let tool: ToolArc = std::sync::Arc::new(FakeMcpTool {
            tool_name: "mcp__test__fake".into(),
        });
        reg.insert("mcp__test__fake".into(), tool);
        reg
    }

    #[tokio::test]
    async fn mcp_tools_visible_to_act_agent() {
        let mock = Arc::new(
            MockChatClient::new().with_default(vec![LlmEvent::Completed {
                text: "done".into(),
                tool_calls: vec![],
                usage: None,
            }]),
        );
        let client = mock.clone() as Arc<dyn ChatStream>;
        // The registry alone is not enough: the server must be enabled in the
        // session config (parent scope) for its tools to be surfaced.
        let mut config = Config::default();
        config.mcp_servers.insert(
            "test".into(),
            opencoder_core::config::McpServerConfig {
                enabled: true,
                inject_to: opencoder_core::InjectionTarget::parent_only(),
                ..Default::default()
            },
        );
        let session = session_for("act", config, client);
        let registry = registry_with_mcp();
        let _ = run_one_llm_call(&session, &registry, &mut |_| {}).await;

        let reqs = mock.requests();
        assert_eq!(reqs.len(), 1);
        let tool_names: Vec<&str> = reqs[0]
            .tools
            .iter()
            .filter_map(|t| t["function"]["name"].as_str())
            .collect();
        assert!(
            tool_names.contains(&"mcp__test__fake"),
            "MCP tool should be visible to act agent: {tool_names:?}"
        );
    }

    #[tokio::test]
    async fn mcp_tools_hidden_from_subagent() {
        let mock = Arc::new(
            MockChatClient::new().with_default(vec![LlmEvent::Completed {
                text: "done".into(),
                tool_calls: vec![],
                usage: None,
            }]),
        );
        let client = mock.clone() as Arc<dyn ChatStream>;
        let agent = resolve_agent("explore").unwrap();
        let session = SessionState::new(
            "test-subagent",
            agent,
            Config::default(),
            client,
            std::env::temp_dir(),
        );
        let registry = registry_with_mcp();
        let _ = run_one_llm_call(&session, &registry, &mut |_| {}).await;

        let reqs = mock.requests();
        assert_eq!(reqs.len(), 1);
        let tool_names: Vec<&str> = reqs[0]
            .tools
            .iter()
            .filter_map(|t| t["function"]["name"].as_str())
            .collect();
        assert!(
            !tool_names.iter().any(|n| n.starts_with("mcp__")),
            "MCP tools must be hidden from subagents: {tool_names:?}"
        );
    }

    /// Config fixture: one CLI registration injected to `explore` only.
    fn explore_only_cli_config() -> Config {
        let mut config = Config::default();
        config.cli.insert(
            "test-cli".into(),
            opencoder_core::CliConfig {
                enabled: true,
                inject_to: opencoder_core::InjectionTarget {
                    parent: false,
                    explore: true,
                    build: false,
                },
                content: "EXPLORE_ONLY_CONTRACT".into(),
            },
        );
        config
    }

    fn session_for(agent_name: &str, config: Config, client: Arc<dyn ChatStream>) -> SessionState {
        SessionState::new(
            "test-inject-target",
            resolve_agent(agent_name).unwrap(),
            config,
            client,
            std::env::temp_dir(),
        )
    }

    async fn request_body_for(agent_name: &str, config: Config) -> String {
        let mock = Arc::new(
            MockChatClient::new().with_default(vec![LlmEvent::Completed {
                text: "done".into(),
                tool_calls: vec![],
                usage: None,
            }]),
        );
        let client = mock.clone() as Arc<dyn ChatStream>;
        let session = session_for(agent_name, config, client);
        let registry: HashMap<String, ToolArc> = HashMap::new();
        let _ = run_one_llm_call(&session, &registry, &mut |_| {}).await;
        let reqs = mock.requests();
        assert_eq!(reqs.len(), 1);
        reqs[0].to_body().to_string()
    }

    #[tokio::test]
    async fn cli_injected_only_into_explore_subagent_by_name() {
        let body = request_body_for("explore", explore_only_cli_config()).await;
        assert!(body.contains("EXPLORE_ONLY_CONTRACT"));
        let body = request_body_for("build", explore_only_cli_config()).await;
        assert!(
            !body.contains("EXPLORE_ONLY_CONTRACT"),
            "build subagent must not see explore-only CLI: {body}"
        );
        let body = request_body_for("act", explore_only_cli_config()).await;
        assert!(
            !body.contains("EXPLORE_ONLY_CONTRACT"),
            "parent agent must not see explore-only CLI: {body}"
        );
    }

    #[tokio::test]
    async fn mcp_tools_scoped_to_single_subagent_by_name() {
        // Server injected to `build` only: tools surface for build, not explore.
        let mut config = Config::default();
        config.mcp_servers.insert(
            "test".into(),
            opencoder_core::config::McpServerConfig {
                enabled: true,
                inject_to: opencoder_core::InjectionTarget {
                    parent: false,
                    explore: false,
                    build: true,
                },
                ..Default::default()
            },
        );
        let mock = Arc::new(
            MockChatClient::new().with_default(vec![LlmEvent::Completed {
                text: "done".into(),
                tool_calls: vec![],
                usage: None,
            }]),
        );
        let client = mock.clone() as Arc<dyn ChatStream>;
        let session = session_for("build", config.clone(), client);
        let registry = registry_with_mcp();
        let _ = run_one_llm_call(&session, &registry, &mut |_| {}).await;
        let reqs = mock.requests();
        let tool_names: Vec<&str> = reqs[0]
            .tools
            .iter()
            .filter_map(|t| t["function"]["name"].as_str())
            .collect();
        assert!(
            tool_names.contains(&"mcp__test__fake"),
            "build subagent sees its scoped server: {tool_names:?}"
        );

        let mock = Arc::new(
            MockChatClient::new().with_default(vec![LlmEvent::Completed {
                text: "done".into(),
                tool_calls: vec![],
                usage: None,
            }]),
        );
        let client = mock.clone() as Arc<dyn ChatStream>;
        let session = session_for("explore", config, client);
        let _ = run_one_llm_call(&session, &registry, &mut |_| {}).await;
        let reqs = mock.requests();
        let tool_names: Vec<&str> = reqs[0]
            .tools
            .iter()
            .filter_map(|t| t["function"]["name"].as_str())
            .collect();
        assert!(
            !tool_names.iter().any(|n| n.starts_with("mcp__")),
            "explore subagent must not see build-only server: {tool_names:?}"
        );
    }

    #[tokio::test]
    async fn mcp_tools_hidden_from_workflow_agent() {
        let mock = Arc::new(
            MockChatClient::new().with_default(vec![LlmEvent::Completed {
                text: r#"{"operation":"suspend","reason":"test"}"#.into(),
                tool_calls: vec![],
                usage: None,
            }]),
        );
        let client = mock.clone() as Arc<dyn ChatStream>;
        let agent = resolve_agent("workflow").unwrap();
        let mut config = Config::default();
        config.cli.insert(
            "test-cli".into(),
            opencoder_core::CliConfig {
                enabled: true,
                inject_to: opencoder_core::InjectionTarget::parent_only(),
                content: "WORKFLOW_MUST_NOT_SEE_CLI".into(),
            },
        );
        let session =
            SessionState::new("test-workflow", agent, config, client, std::env::temp_dir());
        let registry = registry_with_mcp();
        let _ = run_one_llm_call(&session, &registry, &mut |_| {}).await;

        let reqs = mock.requests();
        assert_eq!(reqs.len(), 1);
        assert!(
            reqs[0].tools.is_empty(),
            "workflow agent must not receive execution tools: {:?}",
            reqs[0].tools
        );
        assert!(
            !reqs[0]
                .to_body()
                .to_string()
                .contains("WORKFLOW_MUST_NOT_SEE_CLI"),
            "workflow agent must not receive registered CLI instructions"
        );
    }
}

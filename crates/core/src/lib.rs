pub mod agent;
pub mod computer_use;
pub mod config;
pub mod error;
pub mod json;
pub mod message;
pub mod net;
pub mod skill;
pub mod sse;
pub mod tool;
pub mod tool_guard_config;

pub use agent::{
    builtin_agents, resolve_agent, strip_tools_subagent_ad, tool_preamble, Agent, AgentKind,
    AgentMode, ToolFilter, TOOLS_SUBAGENT_AD,
};
pub use computer_use::{
    ComputerAction, ComputerUseExecutor, ComputerUseLoop, LoopOutcome, Observation,
    ProviderBackend, RecordingExecutor,
};
pub use config::{
    looks_like_env_var, AgentDefaults, CapabilitiesConfig, CompactionConfig, Config, Endpoint,
    HttpHeader, NetworkConfig, ProviderConfig, DEFAULT_CONTEXT_LIMIT,
};
pub use tool_guard_config::ToolGuardConfig;

pub use error::{CoreError, Result};
pub use message::{ContentBlock, Message, MessageUsage, Role};
pub use net::{build_http_client, effective_proxy};
pub use skill::{
    discover as discover_skills, extract_skill_tokens, seed_builtin_skills, seed_builtin_skills_in,
    seed_dep_gated_skills, seed_dep_gated_skills_in, skills_dir, write_install_script,
    write_install_script_in, Skill, DEPS_SENTINEL,
};
pub use sse::SseEvt;
pub use tool::{Tool, ToolArc, ToolContext, ToolOutput, ToolSchema};

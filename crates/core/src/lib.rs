pub mod agent;
pub mod config;
pub mod data_dir;
pub mod error;
pub mod json;
pub mod message;
pub mod net;
pub mod skill;
pub mod sse;
pub mod tool;
pub mod tool_deps;
pub mod tool_guard_config;
pub mod version;

pub use agent::{
    builtin_agents, resolve_agent, tool_preamble, Agent, AgentKind, AgentMode, ToolFilter,
};
pub use config::envs::{
    active_env, create_env, delete_env, env_dir, envs_home, list_envs, recapture_env,
    set_active_env, validate_env_name,
};
pub use config::{
    looks_like_env_var, scoped_config_home, AgentDefaults, ApMode, AutoPilotConfig, CliConfig,
    CompactionConfig, Config, Endpoint, HttpHeader, InjectionTarget, KeymapConfig, McpServerConfig,
    NetworkConfig, OutputStreamlineConfig, ProviderConfig, ScopedConfigHome, DEFAULT_CONTEXT_LIMIT,
    KEYMAP_INFO,
};
pub use data_dir::{data_dir_for, data_root, workdir_hash};
pub use tool_deps::{all_installed, check_tool_deps, ToolDepStatus};
pub use tool_guard_config::ToolGuardConfig;

pub use error::{CoreError, Result};
pub use message::{ContentBlock, Message, MessageUsage, Role};
pub use net::{build_http_client, effective_proxy};
pub use skill::{
    body_with_source, discover as discover_skills, discover_in, extract_skill_tokens,
    seed_builtin_skills, seed_builtin_skills_in, seed_dep_gated_skills, seed_dep_gated_skills_in,
    skills_dir, strip_resolved_skill_tokens, write_install_script, write_install_script_in, Skill,
    DEPS_SENTINEL,
};
pub use sse::SseEvt;
pub use tool::{Tool, ToolArc, ToolContext, ToolOutput, ToolSchema};

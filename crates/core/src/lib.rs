pub mod agent;
pub mod auth_sig;
pub mod config;
pub mod data_dir;
pub mod error;
pub mod json;
pub mod message;
pub mod net;
pub mod node_protocol;
pub mod share_fs;
#[cfg(test)]
mod share_fs_tests;
pub mod skill;
pub mod sse;
pub mod tool;
pub mod tool_deps;
pub mod tool_guard_config;
pub mod version;

pub use agent::BUILD_DELEGATION_CLAUSE;
pub use agent::{
    build_delegation_hidden, builtin_agents, effective_default_agent, resolve_agent,
    strip_build_delegation, tool_preamble, Agent, AgentKind, AgentMode, ToolFilter,
};
pub use config::envs::{
    active_env, create_env, delete_env, env_dir, envs_home, list_envs, recapture_env,
    set_active_env, set_active_env_checked, validate_env_name,
};
pub use config::{
    looks_like_env_var, scoped_config_home, AgentDefaults, ApMode, AutoPilotConfig, CliConfig,
    CompactionConfig, Config, Endpoint, HttpHeader, InjectionTarget, KeymapConfig, McpServerConfig,
    NetworkConfig, OutputStreamlineConfig, ProviderConfig, ScopedConfigHome, StorageBackend,
    StorageConfig, DEFAULT_CONTEXT_LIMIT, KEYMAP_INFO,
};
pub use data_dir::{data_dir_for, data_root, workdir_hash};
pub use tool_deps::{all_installed, check_tool_deps, ToolDepStatus};
pub use tool_guard_config::ToolGuardConfig;

pub use error::{CoreError, Result};
pub use message::{ContentBlock, Message, MessageUsage, Role};
pub use net::{build_http_client, effective_proxy};
pub use share_fs::{
    agent_tool_path, atomic_write, atomic_write_json, effective_share_dir, env_context_path,
    list_child_dirs, list_child_files, read_json_opt, resolve_tool_ref, set_share_dir_override,
    todo_context_path, todo_dir, todo_env_binding_path, todo_meta_path, todo_version_dir, tool_ref,
    validate_share_name, AGENT_TOOLS_PREFIX,
};
pub use skill::{
    body_with_source, discover as discover_skills, discover_in, extract_skill_tokens,
    seed_builtin_skills, seed_builtin_skills_in, seed_dep_gated_skills, seed_dep_gated_skills_in,
    skills_dir, strip_resolved_skill_tokens, write_install_script, write_install_script_in, Skill,
    DEPS_SENTINEL,
};
pub use sse::SseEvt;
pub use tool::{Tool, ToolArc, ToolContext, ToolOutput, ToolSchema};

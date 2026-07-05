pub mod agent;
pub mod config;
pub mod error;
pub mod json;
pub mod message;
pub mod skill;
pub mod tool;

pub use agent::{Agent, AgentKind, AgentMode, ToolFilter, builtin_agents, resolve_agent};
pub use skill::{discover as discover_skills, skills_dir, Skill};
pub use config::{looks_like_env_var, CompactionConfig, Config, DEFAULT_CONTEXT_LIMIT, ProviderConfig};
pub use error::{CoreError, Result};
pub use message::{ContentBlock, Message, MessageUsage, Role};
pub use tool::{Tool, ToolArc, ToolContext, ToolOutput, ToolSchema};

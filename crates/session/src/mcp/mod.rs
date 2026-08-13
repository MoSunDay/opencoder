//! MCP (Model Context Protocol) client support.
//!
//! Connects to user-configured MCP servers, discovers their tools, and wraps
//! them as regular [`opencoder_core::Tool`] implementations so the LLM can call
//! them via function-calling exactly like builtin tools.
//!
//! Activation contract: MCP is **only** active when the session config has at
//! least one `enabled == true` server.  With no enabled servers there is zero
//! MCP overhead — no connections, no system-prompt injection, no tool
//! registration.

pub mod client;
pub mod pool;
pub mod protocol;
pub mod tool;
pub mod transport;

pub use pool::{cleanup, has_mcp_tools, status_for, sync, tools_for, ConnStatus};
pub use tool::is_mcp_tool;

//! Shared e2e harness: real `build_app` (bearer auth, web assets), a
//! scripted in-process WS node and reqwest helpers. Pure data + functions;
//! all node behaviour is table-driven, no hidden state machines.

pub mod http;
pub mod node;

pub use http::Harness;
pub use node::MockNode;

pub const TOKEN: &str = "e2e-bearer-token";

/// Serializes tests that touch the process-global share/agents dir overrides
/// (todo templates + envs + tools, agent resources) — same pattern as the
/// web crate's `web_todo_templates.rs`.
pub static SHARE_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

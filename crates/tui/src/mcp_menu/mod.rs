//! `/mcp` modal — manage MCP servers (enable/disable, add, edit, delete).
//! Mirrors the `/model` menu. Menus own no I/O; they return a JSON
//! merge-patch the caller persists via `Config::save`.

pub mod form;
pub mod list;
pub mod patch;
pub mod state;
pub mod view;

pub use form::{McpField, McpForm};
pub use list::{McpEntry, McpList};
pub use patch::{delete_mcp_json, save_mcp_json, toggle_mcp_json};
pub use state::{handle_mcp_key, McpMenu, McpOutcome};
pub use view::render_mcp_popup;

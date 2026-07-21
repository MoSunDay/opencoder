//! `/cache_salt` read-only panel: shows the per-agent prefix-cache salt of the
//! main session and every subagent, parent first. Salt format is
//! `<agent_name>:<session_id>` (matches `session::cache_salt_for`), so a vLLM
//! prefix-cache backend can namespace its KV cache per agent/conversation.

pub mod state;
pub mod view;

pub use state::{handle_cache_salt_key, CacheSaltMenu, CacheSaltOutcome};
pub use view::render_cache_salt_popup;

use serde::{Deserialize, Serialize};

/// Tool-failure guard: consecutive-failure threshold with exponential backoff.
/// When the same tool name fails `max_consecutive_failures` times in a row,
/// the current turn is aborted. After each failure an exponential backoff
/// delay (`backoff_base_ms * 2^(n-1)`, capped at `backoff_max_ms`) is applied
/// before continuing, slowing retry loops and giving external resources time
/// to recover. Set `max_consecutive_failures` to 0 to disable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolGuardConfig {
    #[serde(default = "default_tool_failure_threshold")]
    pub max_consecutive_failures: u32,
    #[serde(default = "default_tool_backoff_base")]
    pub backoff_base_ms: u64,
    #[serde(default = "default_tool_backoff_max")]
    pub backoff_max_ms: u64,
}

fn default_tool_failure_threshold() -> u32 {
    3
}
fn default_tool_backoff_base() -> u64 {
    200
}
fn default_tool_backoff_max() -> u64 {
    2000
}

impl Default for ToolGuardConfig {
    fn default() -> Self {
        Self {
            max_consecutive_failures: default_tool_failure_threshold(),
            backoff_base_ms: default_tool_backoff_base(),
            backoff_max_ms: default_tool_backoff_max(),
        }
    }
}

//! Host command tool; Bash-specific timeout inference stays in bash/timeout.rs.
#[cfg(all(test, unix))]
use super::bg::{kill_all, list, test_registry_mutex};
pub use super::command::ShellTool as BashTool;
#[cfg(all(test, unix))]
use super::command::BASH_TIMEOUT_DISPLAY_SECS;
pub(crate) use super::command::{process_group, BASH_TIMEOUT_MARKER, BASH_TIMEOUT_SECS};
#[cfg(all(test, unix))]
use opencoder_core::Tool;
#[cfg(all(test, unix))]
mod tests;

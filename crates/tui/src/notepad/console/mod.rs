//! Vim-style console for the notepad.
//!
//! Replaces the old single-line terminal panel with a modal editor:
//! - **Echo log** (top): read-only scrolling buffer of user prompts, bash
//!   output, and status lines.
//! - **Composer** (bottom): a [`VimState`] multi-line editor supporting
//!   Normal/Insert/Command modes.
//!
//! Submit from Normal-mode `<CR>` or Insert-mode `Alt+Enter`. Lines
//! starting with `!` are run as bash commands; everything else is sent as
//! a prompt to the agent session.

pub mod render;
pub mod state;
pub mod submit;

use crate::vim::{VimMode, VimState};

use crate::notepad::console::state::EchoLog;

/// Console state: echo log + modal composer + status flags.
#[derive(Clone, Debug)]
pub struct ConsoleState {
    /// Read-only echo log.
    pub echo: EchoLog,
    /// Modal multi-line composer (vim engine).
    pub vim: VimState,
    /// `true` while a background bash command is running.
    pub bash_running: bool,
    /// The command currently running (for display).
    pub bash_cmd: Option<String>,
    /// `true` while the agent is processing a submitted prompt.
    pub running: bool,
}

impl ConsoleState {
    pub fn new() -> Self {
        Self {
            echo: EchoLog::new(),
            vim: VimState::new(String::new()),
            bash_running: false,
            bash_cmd: None,
            running: false,
        }
    }

    /// Reset the composer buffer to an empty Insert-mode state.
    pub fn reset_composer(&mut self) {
        self.vim.text.clear();
        self.vim.cursor = 0;
        self.vim.mode = VimMode::Insert;
        self.vim.original.clear();
        self.vim.cmdline.clear();
        self.vim.reset_pending();
    }

    /// Begin a bash command: mark running, echo the command.
    pub fn begin_bash(&mut self, cmd: &str) {
        self.bash_running = true;
        self.bash_cmd = Some(cmd.to_string());
        self.echo.push_bash_cmd(cmd);
    }

    /// Finish a bash command: push output, clear running state.
    pub fn finish_bash(&mut self, output: &str) {
        self.echo.push_bash_out(output);
        self.bash_running = false;
        self.bash_cmd = None;
    }

    /// Sync the agent-running flag from the host loop.
    pub fn set_running(&mut self, running: bool) {
        self.running = running;
    }
}

impl Default for ConsoleState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_in_insert_mode() {
        let c = ConsoleState::new();
        assert_eq!(c.vim.mode, VimMode::Insert);
        assert!(c.echo.is_empty());
        assert!(!c.bash_running);
        assert!(!c.running);
    }

    #[test]
    fn reset_composer_clears_and_sets_insert() {
        let mut c = ConsoleState::new();
        c.vim.text = "hello".into();
        c.vim.mode = VimMode::Normal;
        c.reset_composer();
        assert!(c.vim.text.is_empty());
        assert_eq!(c.vim.cursor, 0);
        assert_eq!(c.vim.mode, VimMode::Insert);
    }

    #[test]
    fn begin_then_finish_bash() {
        let mut c = ConsoleState::new();
        c.begin_bash("echo hi");
        assert!(c.bash_running);
        assert_eq!(c.bash_cmd.as_deref(), Some("echo hi"));
        assert_eq!(c.echo.len(), 1); // command echo
        c.finish_bash("hi");
        assert!(!c.bash_running);
        assert!(c.bash_cmd.is_none());
        assert_eq!(c.echo.len(), 2); // command + output
    }
}

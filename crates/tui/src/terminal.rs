//! Terminal lifecycle: enter alt-screen/raw/mouse/kitty mode on construction,
//! restore it on `Drop` — including from a panic.
//!
//! Previously the setup/teardown lived inline in `app::run`, and the teardown
//! ran only when `run_app` returned normally. A panic anywhere inside the app
//! unwound past the cleanup, leaving the terminal in raw mode + alternate
//! screen + mouse capture: to the user that is indistinguishable from a freeze
//! (last frame frozen, typing has no echo, Ctrl+C/D ineffective) and requires
//! killing the process and often `reset`-ing the shell.
//!
//! The guard makes restoration an RAII invariant: the `Drop` runs on every exit
//! path (normal, `?` error, panic=unwind). A panic hook additionally restores
//! *before* the default hook prints, so a backtrace is readable in the restored
//! terminal rather than buried in the alternate screen.

use anyhow::Result;

use crossterm::cursor::SetCursorStyle;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};

/// RAII handle that holds the terminal in TUI mode. Drop to restore — on any
/// exit path. Construct with [`TerminalGuard::enter`].
pub struct TerminalGuard;

impl TerminalGuard {
    /// Put the terminal into TUI mode (raw + alt-screen + cursor style + mouse
    /// capture + Kitty keyboard enhancement) and install the panic hook.
    pub fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        {
            use crossterm::event::{KeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
            // Best-effort: terminals without the Kitty protocol ignore this.
            let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS;
            let _ = execute!(stdout, PushKeyboardEnhancementFlags(flags));
        }
        execute!(
            stdout,
            EnterAlternateScreen,
            SetCursorStyle::SteadyBar,
            EnableMouseCapture
        )?;

        // Restore the terminal *before* the previous (default) hook prints the
        // panic, so the message/backtrace lands in a sane terminal. Chained to
        // the prior hook so host-installed hooks still run.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            Self::restore();
            prev(info);
        }));

        Ok(TerminalGuard)
    }

    /// Best-effort, idempotent terminal restoration. Every call swallows its
    /// own errors so it is safe to invoke from a panic hook and from `Drop`.
    pub(crate) fn restore() {
        let _ = disable_raw_mode();
        {
            use crossterm::event::PopKeyboardEnhancementFlags;
            let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
        }
        let _ = execute!(std::io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        Self::restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `restore` must be idempotent and panic-free even when the terminal was
    /// never put into raw/alt-screen mode (e.g. running under CI without a
    /// TTY). The panic hook and `run()` rely on calling it unconditionally.
    #[test]
    fn restore_is_idempotent_without_a_tty() {
        TerminalGuard::restore();
        TerminalGuard::restore();
    }
}

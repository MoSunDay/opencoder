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

use std::fmt;

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
        if let Err(e) = execute!(
            stdout,
            EnterAlternateScreen,
            SetCursorStyle::SteadyBar,
            EnableMouseCapture
        ) {
            let _ = disable_raw_mode();
            return Err(e.into());
        }

        // Restore the terminal *before* the previous (default) hook prints the
        // panic, so the message/backtrace lands in a sane terminal. Chained to
        // the prior hook so host-installed hooks still run. The body delegates
        // to `hook_body` so the "restore-then-chain" ordering is unit-testable
        // without constructing a real `PanicInfo`.
        let main_thread = std::thread::current().id();
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if std::thread::current().id() == main_thread {
                Self::hook_body(&Self::restore, &prev, info);
            } else {
                // Worker thread panic: chain to the previous hook without
                // restoring the terminal (main loop may still be rendering).
                prev(info);
            }
        }));

        Ok(TerminalGuard)
    }

    /// Best-effort, idempotent terminal restoration. Every call swallows its
    /// own errors so it is safe to invoke from a panic hook and from `Drop`.
    pub(crate) fn restore() {
        let _ = disable_raw_mode();
        let mut buf = String::new();
        let _ = write_restore(&mut buf);
        let mut out = std::io::stdout();
        let _ = out.write_all(buf.as_bytes());
        let _ = out.flush();
    }

    /// The panic-hook body in isolation: restore the terminal first, then chain
    /// to the previous hook. Generic over the info type `I` so the ordering is
    /// unit-testable with a stand-in (`()`) instead of a real `PanicInfo`.
    fn hook_body<R, P, I>(restore: &R, prev: &P, info: &I)
    where
        R: Fn() + ?Sized,
        P: Fn(&I) + ?Sized,
    {
        restore();
        prev(info);
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        Self::restore();
    }
}

/// Write the ANSI restoration sequences (pop Kitty enhancement, disable mouse
/// capture, leave the alternate screen) to `w`. Single source of truth for what
/// `TerminalGuard::restore` emits — factored out so the exact payload is
/// unit-testable without a real TTY. Targets the unix ANSI path.
fn write_restore<W: fmt::Write>(w: &mut W) -> fmt::Result {
    use crossterm::event::PopKeyboardEnhancementFlags;
    use crossterm::Command;
    PopKeyboardEnhancementFlags.write_ansi(w)?;
    DisableMouseCapture.write_ansi(w)?;
    LeaveAlternateScreen.write_ansi(w)?;
    Ok(())
}

use std::io::Write;

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

    /// The restoration payload must carry the three sequences that reverse the
    /// TUI-mode setup: pop Kitty keyboard enhancement, disable mouse capture,
    /// leave the alternate screen. A missing one leaves the terminal partly
    /// bricked (e.g. mouse still captured, or stuck in alt-screen) — exactly
    /// the "frozen terminal" symptom this guard exists to prevent.
    #[test]
    fn write_restore_emits_all_three_sequences() {
        use crossterm::event::PopKeyboardEnhancementFlags;
        use crossterm::Command;

        // Independent references for each expected sequence.
        let mut want_pop = String::new();
        let _ = PopKeyboardEnhancementFlags.write_ansi(&mut want_pop);
        let mut want_mouse = String::new();
        let _ = DisableMouseCapture.write_ansi(&mut want_mouse);
        let mut want_alt = String::new();
        let _ = LeaveAlternateScreen.write_ansi(&mut want_alt);

        let mut got = String::new();
        write_restore(&mut got).unwrap();

        assert!(got.contains(&want_pop), "missing pop-kitty sequence: {got:?}");
        assert!(
            got.contains(&want_mouse),
            "missing disable-mouse sequence: {got:?}"
        );
        assert!(
            got.contains(&want_alt),
            "missing leave-alt-screen sequence: {got:?}"
        );
    }

    /// The panic hook must restore the terminal *before* chaining to the
    /// previous (default) hook — otherwise the backtrace prints inside the
    /// alternate screen and is unreadable. Verified with stand-in closures.
    #[test]
    fn hook_body_restores_before_chaining_to_prev() {
        let order = std::cell::RefCell::new(Vec::<&str>::new());
        {
            let restore = || order.borrow_mut().push("restore");
            let prev = |_: &()| order.borrow_mut().push("prev");
            TerminalGuard::hook_body(&restore, &prev, &());
        }
        assert_eq!(
            order.into_inner(),
            vec!["restore", "prev"],
            "restore must precede the chained prev hook"
        );
    }
}

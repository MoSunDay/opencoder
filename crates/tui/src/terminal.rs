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
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, KeyCode,
    KeyEvent, KeyEventKind, ModifierKeyCode,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};

/// RAII handle that holds the terminal in TUI mode. Drop to restore — on any
/// exit path. Construct with [`TerminalGuard::enter`].
pub struct TerminalGuard;

impl TerminalGuard {
    /// Put the terminal into TUI mode (raw + alt-screen + cursor style + mouse
    /// capture + Kitty keyboard enhancement + bracketed paste) and install the
    /// panic hook.
    pub fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        {
            use crossterm::event::{KeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
            // Best-effort: terminals without the Kitty protocol ignore this.
            let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES;
            let _ = execute!(stdout, PushKeyboardEnhancementFlags(flags));
        }
        if let Err(e) = execute!(
            stdout,
            EnterAlternateScreen,
            SetCursorStyle::SteadyBar,
            EnableMouseCapture,
            EnableBracketedPaste,
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
            // C1: structured panic tracing for observability.
            tracing::error!(
                panic = %info,
                thread = ?std::thread::current().name(),
                "panic caught by TerminalGuard hook"
            );
            if std::thread::current().id() == main_thread {
                Self::hook_body(&Self::restore, &prev, info);
            } else {
                // C4: Worker thread panic — redirect to a log file instead of
                // calling `prev(info)` (the default hook prints to stderr,
                // which corrupts the alternate-screen terminal display).
                Self::write_panic_log(info);
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

    /// Best-effort redirect of a worker-thread panic message to a log file,
    /// avoiding the stderr output of the default hook that corrupts the
    /// alternate-screen terminal (C4).
    fn write_panic_log(info: &dyn fmt::Display) {
        let mut path = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir);
        path.push("opencoder");
        path.push("tui-panic.log");
        let _ = std::fs::create_dir_all(&path);
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            use std::io::Write;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let _ = writeln!(f, "[{now}] {info}");
        }
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

/// Leave TUI mode temporarily: disable raw mode and emit the inverse of
/// [`TerminalGuard::enter`] so an external process can take over the real
/// terminal with full stdio. Mirrors [`TerminalGuard::restore`] but is
/// callable without owning the guard and surfaces errors.
pub(crate) fn suspend_screen() -> Result<()> {
    disable_raw_mode()?;
    let mut out = std::io::stdout();
    let mut buf = String::new();
    write_restore(&mut buf)?;
    out.write_all(buf.as_bytes())?;
    out.flush()?;
    Ok(())
}

/// Re-enter TUI mode after [`suspend_screen`]: verbatim mirror of
/// [`TerminalGuard::enter`] minus the panic-hook install (still active).
/// The caller should invoke [`ratatui::Terminal::clear`] afterwards so the
/// next draw is a full repaint (ratatui otherwise diffs a stale buffer).
pub(crate) fn resume_screen() -> Result<()> {
    let mut stdout = std::io::stdout();
    {
        use crossterm::event::{KeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
        let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
            | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES;
        let _ = execute!(stdout, PushKeyboardEnhancementFlags(flags));
    }
    execute!(
        stdout,
        EnterAlternateScreen,
        SetCursorStyle::SteadyBar,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    enable_raw_mode()?;
    Ok(())
}

/// Suspend mouse capture so the terminal performs native text selection
/// (hold Shift). Best-effort — callers ignore errors.
pub(crate) fn suspend_mouse_capture() -> Result<()> {
    execute!(std::io::stdout(), DisableMouseCapture)?;
    Ok(())
}

/// Re-enable mouse capture for click interactions (release Shift).
pub(crate) fn resume_mouse_capture() -> Result<()> {
    execute!(std::io::stdout(), EnableMouseCapture)?;
    Ok(())
}

/// Handle a key event for mouse-capture toggling and modifier/release filtering.
///
/// Returns `true` if the event was consumed (Shift toggle, other bare modifier,
/// or key-release) and should NOT be processed further by the app's key handler.
/// Returns `false` for normal key presses that the app should handle.
///
/// When the user holds Shift, mouse capture is suspended so the terminal
/// performs native text selection; releasing Shift restores it.
pub(crate) fn consume_modifier_or_release(k: &KeyEvent, shift_held: &mut bool) -> bool {
    let is_shift = matches!(
        k.code,
        KeyCode::Modifier(ModifierKeyCode::LeftShift | ModifierKeyCode::RightShift)
    );
    if is_shift {
        match k.kind {
            KeyEventKind::Press | KeyEventKind::Repeat => {
                if !*shift_held {
                    *shift_held = true;
                    let _ = suspend_mouse_capture();
                }
            }
            KeyEventKind::Release => {
                if *shift_held {
                    *shift_held = false;
                    let _ = resume_mouse_capture();
                }
            }
        }
        return true;
    }
    // Ignore bare modifier presses (Ctrl, Alt, Super, …).
    if matches!(k.code, KeyCode::Modifier(_)) {
        return true;
    }
    // With REPORT_EVENT_TYPES active, ignore key-release events for all
    // other keys (prevents double-trigger).
    k.kind == KeyEventKind::Release
}

/// Write the ANSI restoration sequences (pop Kitty enhancement, disable mouse
/// capture, disable bracketed paste, leave the alternate screen) to `w`. Single
/// source of truth for what `TerminalGuard::restore` emits — factored out so
/// the exact payload is unit-testable without a real TTY. Targets the unix ANSI
/// path.
fn write_restore<W: fmt::Write>(w: &mut W) -> fmt::Result {
    use crossterm::event::PopKeyboardEnhancementFlags;
    use crossterm::Command;
    PopKeyboardEnhancementFlags.write_ansi(w)?;
    DisableMouseCapture.write_ansi(w)?;
    DisableBracketedPaste.write_ansi(w)?;
    LeaveAlternateScreen.write_ansi(w)?;
    Ok(())
}

use std::io::Write;

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    /// `restore` must be idempotent and panic-free even when the terminal was
    /// never put into raw/alt-screen mode (e.g. running under CI without a
    /// TTY). The panic hook and `run()` rely on calling it unconditionally.
    #[test]
    fn restore_is_idempotent_without_a_tty() {
        TerminalGuard::restore();
        TerminalGuard::restore();
    }

    /// Mouse capture toggle helpers must be safe without a TTY (best-effort).
    #[test]
    fn mouse_capture_toggle_is_safe_without_tty() {
        let _ = suspend_mouse_capture();
        let _ = resume_mouse_capture();
    }

    /// Build a bare shift-key event of the given kind (Left/Right, Press/Repeat/Release).
    fn shift_event(side: ModifierKeyCode, kind: KeyEventKind) -> KeyEvent {
        KeyEvent::new_with_kind(KeyCode::Modifier(side), KeyModifiers::SHIFT, kind)
    }

    /// Shift Left press sets `shift_held = true`; a repeat is idempotent; a
    /// release restores `shift_held = false`. Each event is consumed (true).
    #[test]
    fn consume_modifier_toggle_on_shift_left_press_repeat_release() {
        let mut held = false;

        assert!(consume_modifier_or_release(
            &shift_event(ModifierKeyCode::LeftShift, KeyEventKind::Press),
            &mut held,
        ));
        assert!(held, "Shift Left press must set shift_held = true");

        assert!(consume_modifier_or_release(
            &shift_event(ModifierKeyCode::LeftShift, KeyEventKind::Repeat),
            &mut held,
        ));
        assert!(held, "Shift Left repeat must keep shift_held = true");

        assert!(consume_modifier_or_release(
            &shift_event(ModifierKeyCode::LeftShift, KeyEventKind::Release),
            &mut held,
        ));
        assert!(!held, "Shift Left release must set shift_held = false");
    }

    /// Shift Right mirrors Shift Left for both press and release transitions.
    #[test]
    fn consume_modifier_toggle_on_shift_right_press_release() {
        let mut held = false;

        assert!(consume_modifier_or_release(
            &shift_event(ModifierKeyCode::RightShift, KeyEventKind::Press),
            &mut held,
        ));
        assert!(held);

        assert!(consume_modifier_or_release(
            &shift_event(ModifierKeyCode::RightShift, KeyEventKind::Release),
            &mut held,
        ));
        assert!(!held);
    }

    /// Non-shift bare modifiers (Ctrl, Alt) are consumed but must NOT alter
    /// `shift_held`, whether it was false or already true.
    #[test]
    fn consume_modifier_consumes_non_shift_modifiers_without_state_change() {
        let mut held = false;

        assert!(consume_modifier_or_release(
            &KeyEvent::new_with_kind(
                KeyCode::Modifier(ModifierKeyCode::LeftControl),
                KeyModifiers::CONTROL,
                KeyEventKind::Press,
            ),
            &mut held,
        ));
        assert!(!held, "Ctrl press must not set shift_held");

        held = true;
        assert!(consume_modifier_or_release(
            &KeyEvent::new_with_kind(
                KeyCode::Modifier(ModifierKeyCode::LeftAlt),
                KeyModifiers::ALT,
                KeyEventKind::Press,
            ),
            &mut held,
        ));
        assert!(held, "non-shift modifier must not alter a held shift");
    }

    /// A normal key press (e.g. 'a') passes through (returns false) and never
    /// touches `shift_held`, even when shift is already held.
    #[test]
    fn consume_modifier_passes_through_normal_key_press() {
        let mut held = false;
        assert!(!consume_modifier_or_release(
            &KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
            &mut held,
        ));
        assert!(!held);

        held = true;
        assert!(!consume_modifier_or_release(
            &KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
            &mut held,
        ));
        assert!(held, "normal key must not clear a held shift");
    }

    /// Under REPORT_EVENT_TYPES the Release of a non-shift key is filtered out
    /// (returns true) so the app does not double-trigger, without touching
    /// `shift_held`.
    #[test]
    fn consume_modifier_filters_non_shift_key_release() {
        let mut held = false;
        assert!(consume_modifier_or_release(
            &KeyEvent::new_with_kind(
                KeyCode::Char('a'),
                KeyModifiers::NONE,
                KeyEventKind::Release,
            ),
            &mut held,
        ));
        assert!(!held, "release filtering must not set shift_held");
    }

    /// The restoration payload must carry the four sequences that reverse the
    /// TUI-mode setup: pop Kitty keyboard enhancement, disable mouse capture,
    /// disable bracketed paste, leave the alternate screen. A missing one
    /// leaves the terminal partly bricked (e.g. mouse still captured, or stuck
    /// in alt-screen) — exactly the "frozen terminal" symptom this guard
    /// exists to prevent.
    #[test]
    fn write_restore_emits_all_restoration_sequences() {
        use crossterm::event::PopKeyboardEnhancementFlags;
        use crossterm::Command;

        // Independent references for each expected sequence.
        let mut want_pop = String::new();
        let _ = PopKeyboardEnhancementFlags.write_ansi(&mut want_pop);
        let mut want_mouse = String::new();
        let _ = DisableMouseCapture.write_ansi(&mut want_mouse);
        let mut want_paste = String::new();
        let _ = DisableBracketedPaste.write_ansi(&mut want_paste);
        let mut want_alt = String::new();
        let _ = LeaveAlternateScreen.write_ansi(&mut want_alt);

        let mut got = String::new();
        write_restore(&mut got).unwrap();

        assert!(
            got.contains(&want_pop),
            "missing pop-kitty sequence: {got:?}"
        );
        assert!(
            got.contains(&want_mouse),
            "missing disable-mouse sequence: {got:?}"
        );
        assert!(
            got.contains(&want_paste),
            "missing disable-bracketed-paste sequence: {got:?}"
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

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
    KeyEvent, KeyEventKind, KeyboardEnhancementFlags, ModifierKeyCode,
    PushKeyboardEnhancementFlags,
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
    /// capture + Kitty keyboard enhancement + bracketed paste), install the
    /// panic hook, and arm the process-wide signal guard. The Kitty flags are
    /// pushed strictly *after* entering the alternate screen — see
    /// [`write_enter`] for why the ordering is load-bearing.
    pub fn enter() -> Result<Self> {
        enable_raw_mode()?;
        // Compose the whole setup into one buffer and write it once — mirrors
        // `restore` (also buffered), so two racing writers can never interleave
        // partial escape sequences into terminal garbage.
        let mut setup = String::new();
        // Writing to a String is infallible.
        let _ = write_enter(&mut setup);
        let mut stdout = std::io::stdout();
        if let Err(e) = stdout
            .write_all(setup.as_bytes())
            .and_then(|_| stdout.flush())
        {
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

        // Signal guard: from this millisecond on, a termination signal
        // (SIGHUP/SIGINT/SIGQUIT/SIGTERM) restores the terminal before the
        // process dies — otherwise mouse capture stays enabled in the host
        // terminal and every later click/drag prints escape garbage into the
        // shell. Armed here (not in the liveness supervisor) so the boot
        // window before the supervisor is spawned is covered too. Idempotent
        // process-wide singleton — see `signal_guard`.
        crate::signal_guard::arm_once();

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

/// Whether a Shift release should restore mouse capture: copy mode owns the
/// capture state (suspended on enter, resumed on exit), so a release inside
/// copy mode must not steal native selection back from the terminal.
fn resumes_on_shift_release(copy_mode: bool) -> bool {
    !copy_mode
}

/// Handle a key event for mouse-capture toggling and modifier/release filtering.
///
/// Returns `true` if the event was consumed (Shift toggle, other bare modifier,
/// or key-release) and should NOT be processed further by the app's key handler.
/// Returns `false` for normal key presses that the app should handle.
///
/// When the user holds Shift, mouse capture is suspended so the terminal
/// performs native text selection; releasing Shift restores it. While copy
/// mode is active it owns the capture state (suspended on enter, resumed on
/// exit), so shift transitions must not fight over the terminal — state
/// tracking continues, capture toggling is suppressed.
pub(crate) fn consume_modifier_or_release(
    k: &KeyEvent,
    shift_held: &mut bool,
    copy_mode: bool,
) -> bool {
    let is_shift = matches!(
        k.code,
        KeyCode::Modifier(ModifierKeyCode::LeftShift | ModifierKeyCode::RightShift)
    );
    if is_shift {
        match k.kind {
            KeyEventKind::Press | KeyEventKind::Repeat => {
                if !*shift_held {
                    *shift_held = true;
                    if !copy_mode {
                        let _ = suspend_mouse_capture();
                    }
                }
            }
            KeyEventKind::Release => {
                if *shift_held {
                    *shift_held = false;
                    if resumes_on_shift_release(copy_mode) {
                        let _ = resume_mouse_capture();
                    }
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

/// Kitty keyboard-enhancement flags requested while the TUI owns the terminal.
/// Best-effort: terminals without the protocol ignore the push.
fn kitty_enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
}

/// Write the ANSI setup sequences (enter the alternate screen, cursor style,
/// mouse capture, bracketed paste, push Kitty keyboard enhancement) to `w`.
/// Single source of truth for what `TerminalGuard::enter` emits — factored out
/// so the exact payload and ordering are unit-testable without a real TTY.
/// Targets the unix ANSI path.
///
/// ORDERING IS LOAD-BEARING: the Kitty flags must be pushed *after*
/// `EnterAlternateScreen` (and popped before `LeaveAlternateScreen` in
/// [`write_restore`]). The keyboard-enhancement flag stack is maintained
/// per-screen by spec-conforming terminals (kitty, ghostty, recent wezterm /
/// foot): entering the alternate screen saves the main screen's stack and the
/// alternate screen gets its own state, which is discarded on exit. Pushing on
/// the main screen and popping inside the alternate screen therefore pops the
/// *wrong* stack — the main screen keeps a live
/// `REPORT_ALL_KEYS_AS_ESCAPE_CODES` entry after the app exits, and every key
/// typed into the shell arrives as a raw `CSI <cp>;<mods>:<event> u` sequence
/// (garbage like `0;5:3u`) with all keys apparently dead. Keeping the
/// push/pop bracket strictly inside the alternate-screen session is balanced
/// under both per-screen and global-stack terminal implementations.
fn write_enter<W: fmt::Write>(w: &mut W) -> fmt::Result {
    use crossterm::Command;
    EnterAlternateScreen.write_ansi(w)?;
    SetCursorStyle::SteadyBar.write_ansi(w)?;
    EnableMouseCapture.write_ansi(w)?;
    EnableBracketedPaste.write_ansi(w)?;
    PushKeyboardEnhancementFlags(kitty_enhancement_flags()).write_ansi(w)?;
    Ok(())
}

/// Write the ANSI restoration sequences (pop Kitty enhancement, disable mouse
/// capture, disable bracketed paste, leave the alternate screen) to `w`. The
/// pop precedes `LeaveAlternateScreen` so it lands on the same (alternate)
/// screen the push in [`write_enter`] landed on — see there for why. Single
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
            false
        ));
        assert!(held, "Shift Left press must set shift_held = true");

        assert!(consume_modifier_or_release(
            &shift_event(ModifierKeyCode::LeftShift, KeyEventKind::Repeat),
            &mut held,
            false
        ));
        assert!(held, "Shift Left repeat must keep shift_held = true");

        assert!(consume_modifier_or_release(
            &shift_event(ModifierKeyCode::LeftShift, KeyEventKind::Release),
            &mut held,
            false
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
            false
        ));
        assert!(held);

        assert!(consume_modifier_or_release(
            &shift_event(ModifierKeyCode::RightShift, KeyEventKind::Release),
            &mut held,
            false
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
            false
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
            false
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
            false
        ));
        assert!(!held);

        held = true;
        assert!(!consume_modifier_or_release(
            &KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
            &mut held,
            false
        ));
        assert!(held, "normal key must not clear a held shift");
    }

    /// In copy mode the shift state machine keeps running (events consumed,
    /// `shift_held` tracks correctly) — only the capture toggling is
    /// suppressed, so a Shift release inside copy mode cannot resume mouse
    /// capture and steal native selection back from the terminal.
    #[test]
    fn consume_modifier_tracks_shift_in_copy_mode_without_capture_fight() {
        let mut held = false;

        assert!(consume_modifier_or_release(
            &shift_event(ModifierKeyCode::LeftShift, KeyEventKind::Press),
            &mut held,
            true
        ));
        assert!(held, "copy mode must keep tracking shift state");

        assert!(consume_modifier_or_release(
            &shift_event(ModifierKeyCode::LeftShift, KeyEventKind::Release),
            &mut held,
            true
        ));
        assert!(!held, "release must clear shift even in copy mode");
    }

    /// Pure decision: a Shift release restores capture only outside copy mode.
    #[test]
    fn resumes_on_shift_release_gates_capture_restore() {
        assert!(
            !resumes_on_shift_release(true),
            "copy mode must keep capture suspended"
        );
        assert!(
            resumes_on_shift_release(false),
            "normal release restores capture"
        );
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
            false
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

    /// The setup payload must enter the alternate screen BEFORE pushing the
    /// Kitty keyboard-enhancement flags. Spec-conforming terminals (kitty,
    /// ghostty, recent wezterm/foot) maintain the enhancement-flag stack
    /// per-screen: a push issued on the main screen survives the alt-screen
    /// round-trip, and after exit the shell receives raw `CSI u` sequences for
    /// every keypress (`0;5:3u`-style garbage, keys dead). See `write_enter`.
    #[test]
    fn write_enter_pushes_kitty_only_inside_alt_screen() {
        use crossterm::Command;

        let mut want_alt = String::new();
        let _ = EnterAlternateScreen.write_ansi(&mut want_alt);
        let mut want_push = String::new();
        let _ = PushKeyboardEnhancementFlags(kitty_enhancement_flags()).write_ansi(&mut want_push);
        let mut want_mouse = String::new();
        let _ = EnableMouseCapture.write_ansi(&mut want_mouse);
        let mut want_paste = String::new();
        let _ = EnableBracketedPaste.write_ansi(&mut want_paste);

        let mut got = String::new();
        write_enter(&mut got).unwrap();

        let (alt_at, push_at) = (
            got.find(&want_alt)
                .unwrap_or_else(|| panic!("missing enter-alt-screen sequence: {got:?}")),
            got.find(&want_push)
                .unwrap_or_else(|| panic!("missing push-kitty sequence: {got:?}")),
        );
        assert!(
            alt_at < push_at,
            "Kitty push must come after entering the alternate screen: {got:?}"
        );
        assert!(
            got.contains(&want_mouse),
            "missing enable-mouse sequence: {got:?}"
        );
        assert!(
            got.contains(&want_paste),
            "missing enable-bracketed-paste sequence: {got:?}"
        );
    }

    /// The restoration payload must pop the Kitty flags BEFORE leaving the
    /// alternate screen, so the pop balances the push from `write_enter` on
    /// the same (alternate) screen under per-screen-stack terminals. Popping
    /// after `LeaveAlternateScreen` would pop the main screen's stack and
    /// re-introduce the post-exit `CSI u` key leak.
    #[test]
    fn write_restore_pops_kitty_before_leaving_alt_screen() {
        use crossterm::event::PopKeyboardEnhancementFlags;
        use crossterm::Command;

        let mut want_pop = String::new();
        let _ = PopKeyboardEnhancementFlags.write_ansi(&mut want_pop);
        let mut want_alt = String::new();
        let _ = LeaveAlternateScreen.write_ansi(&mut want_alt);

        let mut got = String::new();
        write_restore(&mut got).unwrap();

        let (pop_at, alt_at) = (
            got.find(&want_pop)
                .unwrap_or_else(|| panic!("missing pop-kitty sequence: {got:?}")),
            got.find(&want_alt)
                .unwrap_or_else(|| panic!("missing leave-alt-screen sequence: {got:?}")),
        );
        assert!(
            pop_at < alt_at,
            "Kitty pop must come before leaving the alternate screen: {got:?}"
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

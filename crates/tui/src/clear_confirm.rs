//! Clear-context countdown confirmation (误操作防护).
//!
//! Both triggers of the fold-and-restart path — the Shift+Tab chord and the
//! `/act_clear_context` command (canonical; `/clear_context` is the legacy
//! alias) — arm a short countdown instead of firing immediately. While armed
//! a single transcript marker counts down and the status chip (spinner frame
//! included) animates; the composer stays live so a real submission can be
//! typed during the window, and a submission (`Enter`) — or a second
//! Shift+Tab, the guard's own chord doubling as its confirm — fires early
//! with the typed text folded into the compound rest; `Esc` cancels (回撤) and
//! restores any swallowed draft. Pure state + key/tick decisions —
//! no I/O — so the misop guard is trivially testable.

use std::time::Instant;

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::chat::ChatView;
use crate::render::SPINNER;
use crate::theme;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Countdown window before the armed clear-context fires on its own.
pub(crate) const CLEAR_CONFIRM_WINDOW_MS: u64 = 5_000;

/// Canonical control command — the `act` prefix kept explicit so the command
/// reads as the act-agent fold-and-restart it is. `/clear_context` stays a
/// silent legacy alias (persisted inputs may still carry it).
pub(crate) const CLEAR_CONTEXT_CMD: &str = "/act_clear_context";

/// An armed, pending clear-context confirmation.
#[derive(Debug)]
pub(crate) struct ClearConfirm {
    pub(crate) armed_at: Instant,
    /// Compound rest forwarded after the command (Shift+Tab draft).
    pub(crate) rest: Option<String>,
    /// Draft restored to the composer when the arm is cancelled (回撤).
    pub(crate) restore_draft: Option<String>,
}

/// Outcome of an intercepted key (or an expired window) while armed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfirmFlow {
    /// Enter pressed or window elapsed — execute the clear now.
    Fire,
    /// Esc pressed — drop the arm and restore the draft.
    Cancel,
}

pub(crate) fn arm(rest: Option<String>, restore_draft: Option<String>) -> ClearConfirm {
    ClearConfirm {
        armed_at: Instant::now(),
        rest,
        restore_draft,
    }
}

/// Split a leading `/act_clear_context` (or legacy `/clear_context`) head off
/// `text`, returning the compound rest if any. `None` when the head is not
/// the clear-context command. Unlike `command::parse` this matches compound
/// input, so typed `/act_clear_context <tail>` arms with its tail instead of
/// leaking the raw slash text to the model as a plain prompt.
pub(crate) fn head_rest(text: &str) -> Option<Option<String>> {
    let t = text.trim();
    let raw = t
        .strip_prefix(CLEAR_CONTEXT_CMD)
        .or_else(|| t.strip_prefix("/clear_context"))?;
    // The head must be a complete token: `/act_clear_contextx` is NOT the
    // command — the tail may only be empty or whitespace-led.
    if !raw.is_empty() && !raw.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = raw.trim();
    Some((!rest.is_empty()).then(|| rest.to_string()))
}

/// Full control-command text submitted when the countdown fires.
pub(crate) fn command_text(cc: &ClearConfirm) -> String {
    match cc.rest.as_deref() {
        Some(rest) => format!("{CLEAR_CONTEXT_CMD} {rest}"),
        None => CLEAR_CONTEXT_CMD.to_string(),
    }
}

/// Whole seconds left before the armed clear fires on its own.
pub(crate) fn remaining_secs(cc: &ClearConfirm, now: Instant) -> u64 {
    let elapsed = now.duration_since(cc.armed_at).as_millis() as u64;
    CLEAR_CONFIRM_WINDOW_MS
        .saturating_sub(elapsed)
        .div_ceil(1000)
}

pub(crate) fn expired(cc: &ClearConfirm, now: Instant) -> bool {
    now.duration_since(cc.armed_at).as_millis() as u64 >= CLEAR_CONFIRM_WINDOW_MS
}

/// Countdown status-chip text (rendered through the mode-flash slot). The
/// spinner frame between the arrow and the countdown (reused from the
/// status-bar `SPINNER`) animates with `anim_tick` so the armed guard reads
/// as live while it ticks down.
pub(crate) fn banner(cc: &ClearConfirm, now: Instant, anim_tick: u32) -> String {
    let spin = SPINNER[(anim_tick as usize) % SPINNER.len()];
    format!(
        "\u{2192} {spin} {}s 之后仅保留计划并执行\u{2026}",
        remaining_secs(cc, now)
    )
}

/// Arm the guard and echo one countdown marker into the transcript, then
/// raise the live countdown chip. The `Esc` / `Enter` affordances stay in
/// the help page — the transcript carries no key hints.
pub(crate) fn engage(
    cc: &mut Option<ClearConfirm>,
    chat: &mut ChatView,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    rest: Option<String>,
    restore_draft: Option<String>,
) {
    *cc = Some(arm(rest, restore_draft));
    chat.push_marker(marker_line(format!(
        "[clear] {}s 之后仅保留计划并执行\u{2026}",
        CLEAR_CONFIRM_WINDOW_MS / 1000
    )));
    if let Some(a) = cc.as_ref() {
        *mode_flash = Some((banner(a, Instant::now(), anim_tick), anim_tick));
    }
}

/// Convenience wrapper: arm from raw text when it heads with the
/// clear-context command (either spelling). Returns `true` when armed.
pub(crate) fn maybe_arm(
    cc: &mut Option<ClearConfirm>,
    chat: &mut ChatView,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
    text: &str,
    restore_draft: Option<String>,
) -> bool {
    match head_rest(text) {
        Some(rest) => {
            engage(cc, chat, mode_flash, anim_tick, rest, restore_draft);
            true
        }
        None => false,
    }
}

/// Keys stay live while armed: a submission (`Enter`) — or a second
/// Shift+Tab (`BackTab`/Tab+SHIFT, the chord that armed the guard;
/// Ctrl/Alt/Super chord variants stay inert) — fires early —
/// the composer text typed during the window merges into the compound rest first
/// (see [`merge_typed`], applied by the caller) — `Esc` cancels (回撤 —
/// restores the swallowed draft), plain composer editing (chars / Backspace /
/// Left / Right, Shift|Alt+Enter newline) keeps working so the task to run
/// after the fold can be typed during the window, everything else is inert.
/// Returns `None` when the key produced no flow change. On `Fire` the arm is
/// left in place — the caller takes it to execute.
pub(crate) fn intercept(
    cc: &mut Option<ClearConfirm>,
    input: &mut String,
    cursor_idx: &mut usize,
    undo_state: &mut crate::undo::UndoState,
    k: KeyEvent,
) -> Option<ConfirmFlow> {
    match k.code {
        KeyCode::Enter => {
            // Shift+Enter / Alt+Enter still insert a newline (multi-line
            // input) instead of firing — same as the live composer.
            if k.modifiers
                .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT)
            {
                let (s, i) = crate::composer::insert_newline(input, *cursor_idx);
                *input = s;
                *cursor_idx = i;
                crate::undo::snapshot(undo_state, input, *cursor_idx, false);
                return None;
            }
            Some(ConfirmFlow::Fire)
        }
        // A second Shift+Tab is an explicit "go now" — the chord that armed
        // the guard doubles as its confirm. BackTab arrives with the SHIFT
        // flag stripped on some terminals; Tab+SHIFT is the same chord.
        // CONTROL/ALT/SUPER chord variants are never the confirm: the retired
        // ctrl+shift+tab lands here as BackTab+CONTROL|SHIFT (pane-switch
        // style), and letting it fire would turn a chord that used to be a
        // harmless mode switch into an immediate destructive clear.
        KeyCode::BackTab
            if !k
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER) =>
        {
            Some(ConfirmFlow::Fire)
        }
        KeyCode::Tab
            if k.modifiers.contains(KeyModifiers::SHIFT)
                && !k.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
        {
            Some(ConfirmFlow::Fire)
        }
        KeyCode::Esc => {
            let armed = cc.take();
            if let Some(draft) = armed.as_ref().and_then(|a| a.restore_draft.clone()) {
                *input = draft;
                *cursor_idx = input.len();
                crate::undo::reset(undo_state, input, *cursor_idx);
            }
            Some(ConfirmFlow::Cancel)
        }
        KeyCode::Backspace => {
            if let Some((s, i)) = crate::composer::backspace(input, *cursor_idx) {
                *input = s;
                *cursor_idx = i;
                crate::undo::snapshot(undo_state, input, *cursor_idx, false);
            }
            None
        }
        KeyCode::Left => {
            *cursor_idx = cursor_idx.saturating_sub(1);
            None
        }
        KeyCode::Right => {
            *cursor_idx = (*cursor_idx + 1).min(input.chars().count());
            None
        }
        KeyCode::Char(c) => {
            // Alt+Char (tmux escape-time Esc merge) and Ctrl combos never
            // reach the input box — same ghost-garbage guard as the composer.
            if k.modifiers
                .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
            {
                return None;
            }
            let (s, i) = crate::composer::insert_char(input, *cursor_idx, c);
            *input = s;
            *cursor_idx = i;
            crate::undo::snapshot(undo_state, input, *cursor_idx, true);
            None
        }
        _ => None,
    }
}

/// Fold the composer text typed during the countdown into the compound rest
/// before firing: a re-typed clear-context command supersedes (its tail
/// wins), any other text appends to the armed rest. Blank input leaves the
/// arm untouched — a bare Enter just fires what was armed.
pub(crate) fn merge_typed(cc: &mut ClearConfirm, typed: &str) {
    let typed = typed.trim();
    if typed.is_empty() {
        return;
    }
    if let Some(rest) = head_rest(typed) {
        cc.rest = rest;
        return;
    }
    cc.rest = Some(match cc.rest.take() {
        Some(armed) => format!("{armed} {typed}"),
        None => typed.to_string(),
    });
}

/// Tick the armed confirm: refresh the countdown chip so it outlives the
/// mode-flash lifetime, and hand back the arm once the window elapsed (the
/// caller then fires it). `None` = keep waiting (or nothing armed).
pub(crate) fn tick(
    cc: &mut Option<ClearConfirm>,
    mode_flash: &mut Option<(String, u32)>,
    anim_tick: u32,
) -> Option<ClearConfirm> {
    let armed = cc.as_ref()?;
    *mode_flash = Some((banner(armed, Instant::now(), anim_tick), anim_tick));
    if expired(armed, Instant::now()) {
        cc.take()
    } else {
        None
    }
}

/// Cancel feedback: the fold was withdrawn (回撤) — nothing was lost.
pub(crate) fn push_cancel_marker(chat: &mut ChatView) {
    chat.push_marker(marker_line("[clear] 已取消（回撤）— 上下文未清空".into()));
}

fn marker_line(text: String) -> Line<'static> {
    Line::from(Span::styled(text, Style::default().fg(theme::warn_color())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use std::time::Duration;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    #[test]
    fn head_rest_matches_canonical_and_legacy_compound() {
        assert_eq!(head_rest("/act_clear_context"), Some(None));
        assert_eq!(head_rest("/clear_context"), Some(None));
        assert_eq!(
            head_rest("/act_clear_context now run checks"),
            Some(Some("now run checks".into()))
        );
        assert_eq!(
            head_rest("/clear_context  spaced tail "),
            Some(Some("spaced tail".into()))
        );
        assert_eq!(head_rest("/act"), None);
        assert_eq!(head_rest("clear_context"), None);
        assert_eq!(head_rest("/act_clear_contextx"), None);
    }

    #[test]
    fn command_text_appends_compound_rest() {
        let bare = arm(None, None);
        assert_eq!(command_text(&bare), "/act_clear_context");
        let compound = arm(Some("finish the summary".into()), None);
        assert_eq!(
            command_text(&compound),
            "/act_clear_context finish the summary"
        );
    }

    #[test]
    fn remaining_and_expiry_track_the_window() {
        let armed_at = Instant::now() - Duration::from_millis(2_000);
        let cc = ClearConfirm {
            armed_at,
            rest: None,
            restore_draft: None,
        };
        assert_eq!(remaining_secs(&cc, Instant::now()), 3);
        assert!(!expired(&cc, Instant::now()));
        let spent = ClearConfirm {
            armed_at: Instant::now() - Duration::from_millis(CLEAR_CONFIRM_WINDOW_MS),
            rest: None,
            restore_draft: None,
        };
        assert_eq!(remaining_secs(&spent, Instant::now()), 0);
        assert!(expired(&spent, Instant::now()));
    }

    #[test]
    fn intercept_enter_fires_and_leaves_arm_for_caller() {
        let mut cc = Some(arm(None, None));
        let mut input = String::new();
        let mut cursor = 0;
        let mut undo = crate::undo::init(&input, cursor);
        assert_eq!(
            intercept(
                &mut cc,
                &mut input,
                &mut cursor,
                &mut undo,
                key(KeyCode::Enter)
            ),
            Some(ConfirmFlow::Fire)
        );
        assert!(cc.is_some(), "caller takes the arm to execute");
    }

    #[test]
    fn intercept_esc_cancels_and_restores_the_draft() {
        let mut cc = Some(arm(None, Some("draft text".into())));
        let mut input = String::new();
        let mut cursor = 0;
        let mut undo = crate::undo::init(&input, cursor);
        assert_eq!(
            intercept(
                &mut cc,
                &mut input,
                &mut cursor,
                &mut undo,
                key(KeyCode::Esc)
            ),
            Some(ConfirmFlow::Cancel)
        );
        assert!(cc.is_none());
        assert_eq!(input, "draft text");
        assert_eq!(cursor, "draft text".len());
    }

    #[test]
    fn intercept_editing_keys_stay_live_others_inert() {
        let mut cc = Some(arm(None, None));
        let mut input = String::new();
        let mut cursor = 0;
        let mut undo = crate::undo::init(&input, cursor);
        // Typing lands in the composer so a real submission can be made
        // during the window.
        assert_eq!(
            intercept(
                &mut cc,
                &mut input,
                &mut cursor,
                &mut undo,
                key(KeyCode::Char('x'))
            ),
            None
        );
        assert_eq!(
            intercept(
                &mut cc,
                &mut input,
                &mut cursor,
                &mut undo,
                key(KeyCode::Char('y'))
            ),
            None
        );
        assert_eq!(input, "xy");
        assert_eq!(cursor, 2);
        assert!(cc.is_some(), "editing must not disturb the arm");
        assert_eq!(
            intercept(
                &mut cc,
                &mut input,
                &mut cursor,
                &mut undo,
                key(KeyCode::Backspace)
            ),
            None
        );
        assert_eq!(input, "x");
        assert_eq!(cursor, 1);
        // Non-editing chords stay inert (swallowed, no flow change).
        for code in [KeyCode::Up, KeyCode::Tab] {
            assert_eq!(
                intercept(&mut cc, &mut input, &mut cursor, &mut undo, key(code)),
                None
            );
        }
        assert_eq!(input, "x", "inert keys must not edit");
        assert!(cc.is_some());
        // Alt/Ctrl combos never reach the composer (ghost-garbage guard).
        let alt_x = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT);
        let ctrl_x = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        for k in [alt_x, ctrl_x] {
            assert_eq!(
                intercept(&mut cc, &mut input, &mut cursor, &mut undo, k),
                None
            );
        }
        assert_eq!(input, "x", "Alt/Ctrl combos must stay inert");
    }

    #[test]
    fn intercept_shift_enter_inserts_newline_instead_of_firing() {
        let mut cc = Some(arm(None, None));
        let mut input = String::from("ab");
        let mut cursor = 2;
        let mut undo = crate::undo::init(&input, cursor);
        let shift_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
        assert_eq!(
            intercept(&mut cc, &mut input, &mut cursor, &mut undo, shift_enter),
            None,
            "Shift+Enter is a newline, not a submission"
        );
        assert_eq!(input, "ab\n");
        assert_eq!(cursor, 3);
        assert!(cc.is_some(), "the arm must survive a newline insert");
    }

    #[test]
    fn intercept_shift_tab_repress_fires_like_submit() {
        let mut cc = Some(arm(None, None));
        let mut input = String::new();
        let mut cursor = 0;
        let mut undo = crate::undo::init(&input, cursor);
        // The same chord that armed the guard confirms it.
        assert_eq!(
            intercept(
                &mut cc,
                &mut input,
                &mut cursor,
                &mut undo,
                key(KeyCode::BackTab)
            ),
            Some(ConfirmFlow::Fire)
        );
        assert!(cc.is_some(), "the arm stays for the caller to take");
        // Tab+SHIFT is the same chord (BackTab ≡ Tab+SHIFT).
        let tab_shift = KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT);
        assert_eq!(
            intercept(&mut cc, &mut input, &mut cursor, &mut undo, tab_shift),
            Some(ConfirmFlow::Fire)
        );
        assert_eq!(input, "", "the confirm chord must not edit");
        // Plain Tab (follow-up/submit) stays inert while armed.
        assert_eq!(
            intercept(
                &mut cc,
                &mut input,
                &mut cursor,
                &mut undo,
                key(KeyCode::Tab)
            ),
            None
        );
        assert!(cc.is_some());
    }

    #[test]
    fn intercept_ctrl_alt_shift_tab_chords_stay_inert() {
        let mut cc = Some(arm(None, None));
        let mut input = String::new();
        let mut cursor = 0;
        let mut undo = crate::undo::init(&input, cursor);
        // Ctrl/Alt chord variants are terminal pane-switch style — never the
        // guard's confirm, so they fall through inert with the arm intact.
        let ctrl_backtab = KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(
            intercept(&mut cc, &mut input, &mut cursor, &mut undo, ctrl_backtab),
            None
        );
        let alt_tab_shift = KeyEvent::new(KeyCode::Tab, KeyModifiers::ALT | KeyModifiers::SHIFT);
        assert_eq!(
            intercept(&mut cc, &mut input, &mut cursor, &mut undo, alt_tab_shift),
            None
        );
        // Super (cmd/win) mutations are filtered the same way.
        let super_backtab =
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SUPER | KeyModifiers::SHIFT);
        assert_eq!(
            intercept(&mut cc, &mut input, &mut cursor, &mut undo, super_backtab),
            None
        );
        let super_tab_shift =
            KeyEvent::new(KeyCode::Tab, KeyModifiers::SUPER | KeyModifiers::SHIFT);
        assert_eq!(
            intercept(&mut cc, &mut input, &mut cursor, &mut undo, super_tab_shift),
            None
        );
        assert!(cc.is_some(), "inert chords must not drop the arm");
        assert_eq!(input, "", "inert chords must not edit the composer");
        assert_eq!(cursor, 0);
    }

    #[test]
    fn merge_typed_appends_supersedes_and_ignores_blank() {
        // Blank input leaves the arm untouched.
        let mut cc = arm(Some("armed rest".into()), None);
        merge_typed(&mut cc, "   ");
        assert_eq!(cc.rest.as_deref(), Some("armed rest"));
        // Plain text appends to the armed rest.
        merge_typed(&mut cc, "and lint");
        assert_eq!(cc.rest.as_deref(), Some("armed rest and lint"));
        // No armed rest: typed text becomes the rest.
        let mut bare = arm(None, None);
        merge_typed(&mut bare, "run the checks");
        assert_eq!(bare.rest.as_deref(), Some("run the checks"));
        // A re-typed clear-context command supersedes — its tail wins.
        let mut retype = arm(Some("stale".into()), None);
        merge_typed(&mut retype, "/act_clear_context fresh tail");
        assert_eq!(retype.rest.as_deref(), Some("fresh tail"));
        // Legacy spelling supersedes too; bare re-submit clears the rest.
        let mut legacy = arm(Some("stale".into()), None);
        merge_typed(&mut legacy, "/clear_context");
        assert_eq!(legacy.rest, None);
    }

    #[test]
    fn maybe_arm_arms_only_clear_context_text() {
        let mut cc: Option<ClearConfirm> = None;
        let mut chat = ChatView::default();
        let mut flash: Option<(String, u32)> = None;
        assert!(!maybe_arm(&mut cc, &mut chat, &mut flash, 0, "/act", None));
        assert!(cc.is_none());
        assert!(maybe_arm(
            &mut cc,
            &mut chat,
            &mut flash,
            0,
            "/clear_context tail",
            Some("t".into())
        ));
        let armed = cc.expect("legacy spelling must arm too");
        assert_eq!(armed.rest, Some("tail".into()));
        assert_eq!(armed.restore_draft, Some("t".into()));
        let (chip, _) = flash.expect("countdown chip must be raised");
        assert!(chip.contains("之后仅保留计划并执行"), "chip: {chip}");
        let markers: Vec<String> = chat
            .blocks
            .iter()
            .filter_map(|b| match b {
                crate::chat::ChatBlock::Marker(lines) => {
                    Some(lines.iter().map(|l| l.to_string()).collect::<Vec<_>>())
                }
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(
            markers.len(),
            1,
            "exactly one countdown marker: {markers:?}"
        );
        assert!(
            markers[0].contains("5s 之后仅保留计划并执行"),
            "countdown marker: {:?}",
            markers[0]
        );
    }
}

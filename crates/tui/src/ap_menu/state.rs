//! `/ap` mode-picker state: ↑/↓ move (wrap), Enter arms the "save as
//! default?" prompt (`confirm = Some(mode)`), then `y`/Enter saves globally,
//! `n` applies session-only, Esc cancels; Ctrl-D hard-aborts. Mirrors
//! `model_menu::list::handle_key`'s `confirm_save_default` sub-state and
//! `skill_menu::state::handle_skill_key`'s `slot.take()` pattern so
//! `Option<ApMenu>` is never double-borrowed.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::{ApMode, Config};

use super::list::{mode_index, AP_CHOICES};

/// The `/ap` modal. `app.rs` holds `Option<ApMenu>`; while `Some`, all keys
/// route through [`handle_ap_key`].
pub struct ApMenu {
    /// Index into `AP_CHOICES` — the highlighted (cursor) row.
    pub selected: usize,
    /// The mode active when the menu opened — rendered with a `← 当前` mark.
    pub current: ApMode,
    /// `Some(mode)` = the "save as default?" prompt is armed for `mode`
    /// (Enter was pressed on its row); the next keystroke resolves it
    /// (`y`/Enter global, `n` session-only, Esc cancel).
    pub confirm: Option<ApMode>,
}

impl ApMenu {
    /// Pure constructor: the cursor starts on the config's current mode.
    pub fn new(config: &Config) -> Self {
        Self {
            selected: mode_index(config.autopilot.mode),
            current: config.autopilot.mode,
            confirm: None,
        }
    }
}

/// Outcome of a keystroke while the `/ap` modal is open.
pub enum ApOutcome {
    Idle,
    /// Persist as the new GLOBAL default (`Config::save` + reload).
    Save(ApMode),
    /// Apply SESSION-ONLY: pin the override + `sessions.autopilot_mode`.
    SaveSessionOnly(ApMode),
    Cancel,
}

/// Move the cursor `delta` rows, wrapping around the fixed choices.
fn moved(menu: ApMenu, delta: isize) -> ApMenu {
    let n = AP_CHOICES.len() as isize;
    ApMenu {
        selected: (menu.selected as isize + delta).rem_euclid(n) as usize,
        ..menu
    }
}

/// Handle one keystroke while the `/ap` modal is open. Ctrl-D always closes
/// (hard-abort escape hatch for stuck modals, mirroring `handle_skill_key`).
pub fn handle_ap_key(slot: &mut Option<ApMenu>, k: KeyEvent) -> ApOutcome {
    if k.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(k.code, KeyCode::Char('d') | KeyCode::Char('\u{4}'))
    {
        *slot = None;
        return ApOutcome::Cancel;
    }
    let Some(mut menu) = slot.take() else {
        return ApOutcome::Idle;
    };

    // Save-as-default confirmation sub-state (clone of `/model`'s
    // `confirm_save_default` in `model_menu/list.rs`): while armed it takes
    // priority over every list action.
    if let Some(mode) = menu.confirm.take() {
        let (outcome, next) = match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                // y/Enter: persist as the new GLOBAL default. Enter is the
                // natural "confirm" key, matching the dialog's promise.
                (ApOutcome::Save(mode), None)
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                // n: apply session-only, without touching the config files.
                (ApOutcome::SaveSessionOnly(mode), None)
            }
            KeyCode::Esc => {
                // Esc: dismiss the prompt without switching anything.
                (ApOutcome::Cancel, None)
            }
            _ => {
                // Any other key re-arms the prompt and stays idle.
                menu.confirm = Some(mode);
                (ApOutcome::Idle, Some(menu))
            }
        };
        *slot = next;
        return outcome;
    }

    let (outcome, next) = match k.code {
        KeyCode::Up => (ApOutcome::Idle, Some(moved(menu, -1))),
        KeyCode::Down => (ApOutcome::Idle, Some(moved(menu, 1))),
        KeyCode::Enter => {
            // Arm the "save as default?" prompt instead of saving at once.
            menu.confirm = Some(AP_CHOICES[menu.selected].mode);
            (ApOutcome::Idle, Some(menu))
        }
        KeyCode::Esc => (ApOutcome::Cancel, None),
        _ => (ApOutcome::Idle, Some(menu)),
    };
    *slot = next;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_d() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)
    }

    fn cfg_with(mode: ApMode) -> Config {
        let mut config = Config::default();
        config.autopilot.mode = mode;
        config
    }

    /// `ApMenu::new` highlights the config's current mode.
    #[test]
    fn new_highlights_current_mode() {
        assert_eq!(ApMenu::new(&cfg_with(ApMode::Off)).selected, 0);
        assert_eq!(ApMenu::new(&cfg_with(ApMode::Ap)).selected, 1);
        let review = ApMenu::new(&cfg_with(ApMode::Review));
        assert_eq!(review.selected, 2);
        assert_eq!(
            review.current,
            ApMode::Review,
            "current mode is remembered for view"
        );
        assert_eq!(review.confirm, None, "prompt starts unarmed");
    }

    /// ↑/↓ move with wrap-around and keep the menu open (Idle).
    #[test]
    fn up_down_move_with_wrap() {
        let mut slot = Some(ApMenu::new(&cfg_with(ApMode::Ap)));
        assert!(matches!(
            handle_ap_key(&mut slot, key(KeyCode::Down)),
            ApOutcome::Idle
        ));
        assert_eq!(slot.as_ref().unwrap().selected, 2, "down: ap -> review");
        assert!(matches!(
            handle_ap_key(&mut slot, key(KeyCode::Down)),
            ApOutcome::Idle
        ));
        assert_eq!(
            slot.as_ref().unwrap().selected,
            0,
            "down from review wraps to off"
        );
        assert!(matches!(
            handle_ap_key(&mut slot, key(KeyCode::Up)),
            ApOutcome::Idle
        ));
        assert_eq!(
            slot.as_ref().unwrap().selected,
            2,
            "up from off wraps to review"
        );
        assert!(slot.is_some(), "movement keeps the menu open");
    }

    /// Enter arms the "save as default?" prompt instead of saving at once:
    /// the menu stays open with `confirm = Some(highlighted mode)`, and a
    /// following `y` resolves it to a global `Save`.
    #[test]
    fn enter_arms_confirm_then_y_saves_global() {
        let mut slot = Some(ApMenu::new(&cfg_with(ApMode::Off)));
        handle_ap_key(&mut slot, key(KeyCode::Up)); // wrap onto review
        assert!(
            matches!(
                handle_ap_key(&mut slot, key(KeyCode::Enter)),
                ApOutcome::Idle
            ),
            "Enter arms the prompt instead of saving"
        );
        let menu = slot.as_ref().expect("menu stays open while prompting");
        assert_eq!(menu.confirm, Some(ApMode::Review));
        match handle_ap_key(&mut slot, key(KeyCode::Char('y'))) {
            ApOutcome::Save(mode) => assert_eq!(mode, ApMode::Review),
            _ => panic!("expected Save"),
        }
        assert!(slot.is_none(), "y closes the menu");
    }

    /// Enter inside the armed prompt is equivalent to `y`: it saves the
    /// armed mode as the global default (matching the dialog's promise).
    #[test]
    fn enter_in_confirm_also_saves_global() {
        let mut slot = Some(ApMenu::new(&cfg_with(ApMode::Off)));
        handle_ap_key(&mut slot, key(KeyCode::Down)); // off -> ap
        handle_ap_key(&mut slot, key(KeyCode::Enter)); // arm the prompt
        match handle_ap_key(&mut slot, key(KeyCode::Enter)) {
            ApOutcome::Save(mode) => assert_eq!(mode, ApMode::Ap),
            _ => panic!("expected Save"),
        }
        assert!(slot.is_none(), "Enter closes the menu");
    }

    /// `n` inside the armed prompt applies the mode session-only: the menu
    /// closes with `SaveSessionOnly` and nothing is persisted globally.
    #[test]
    fn enter_then_n_saves_session_only() {
        let mut slot = Some(ApMenu::new(&cfg_with(ApMode::Off)));
        handle_ap_key(&mut slot, key(KeyCode::Down)); // off -> ap
        handle_ap_key(&mut slot, key(KeyCode::Enter)); // arm the prompt
        match handle_ap_key(&mut slot, key(KeyCode::Char('n'))) {
            ApOutcome::SaveSessionOnly(mode) => assert_eq!(mode, ApMode::Ap),
            _ => panic!("expected SaveSessionOnly"),
        }
        assert!(slot.is_none(), "n closes the menu");
    }

    /// Esc inside the armed prompt cancels the whole flow: no mode is
    /// applied and the menu closes.
    #[test]
    fn esc_from_confirm_cancels() {
        let mut slot = Some(ApMenu::new(&cfg_with(ApMode::Ap)));
        handle_ap_key(&mut slot, key(KeyCode::Enter)); // arm on ap
        assert!(
            matches!(
                handle_ap_key(&mut slot, key(KeyCode::Esc)),
                ApOutcome::Cancel
            ),
            "expected Cancel from the armed prompt"
        );
        assert!(slot.is_none(), "Esc closes the menu");
    }

    /// Unmapped keys inside the armed prompt re-arm it: the menu stays open,
    /// the armed mode and the cursor row are untouched.
    #[test]
    fn other_key_re_arms_confirm_and_keeps_menu() {
        let mut slot = Some(ApMenu::new(&cfg_with(ApMode::Off)));
        handle_ap_key(&mut slot, key(KeyCode::Down)); // off -> ap
        handle_ap_key(&mut slot, key(KeyCode::Enter)); // arm on ap
        assert!(matches!(
            handle_ap_key(&mut slot, key(KeyCode::Char('x'))),
            ApOutcome::Idle
        ));
        let menu = slot.as_ref().expect("menu stays open on unmapped key");
        assert_eq!(menu.confirm, Some(ApMode::Ap), "prompt stays armed");
        assert_eq!(menu.selected, 1, "cursor unmoved while confirming");
        // Movement keys are swallowed by the prompt too (re-armed, no move).
        assert!(matches!(
            handle_ap_key(&mut slot, key(KeyCode::Up)),
            ApOutcome::Idle
        ));
        assert_eq!(slot.as_ref().unwrap().confirm, Some(ApMode::Ap));
        assert_eq!(slot.as_ref().unwrap().selected, 1);
    }

    /// Esc and Ctrl-D cancel: no patch, menu closed.
    #[test]
    fn esc_and_ctrl_d_cancel_and_close() {
        for k in [key(KeyCode::Esc), ctrl_d()] {
            let mut slot = Some(ApMenu::new(&cfg_with(ApMode::Ap)));
            assert!(
                matches!(handle_ap_key(&mut slot, k), ApOutcome::Cancel),
                "expected Cancel"
            );
            assert!(slot.is_none(), "cancel closes the menu");
        }
    }

    /// Any keystroke on an empty slot is a no-op (Idle).
    #[test]
    fn empty_slot_is_idle() {
        let mut slot: Option<ApMenu> = None;
        for code in [
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Up,
            KeyCode::Char('x'),
        ] {
            assert!(matches!(
                handle_ap_key(&mut slot, key(code)),
                ApOutcome::Idle
            ));
        }
        assert!(slot.is_none());
    }

    /// Unmapped keys keep the menu open without moving the cursor.
    #[test]
    fn unmapped_key_is_idle_and_keeps_menu() {
        let mut slot = Some(ApMenu::new(&cfg_with(ApMode::Ap)));
        assert!(matches!(
            handle_ap_key(&mut slot, key(KeyCode::Char('x'))),
            ApOutcome::Idle
        ));
        assert_eq!(slot.as_ref().unwrap().selected, 1, "cursor unmoved");
    }
}

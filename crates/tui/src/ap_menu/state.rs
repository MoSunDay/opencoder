//! `/ap` mode-picker state: ↑/↓ move (wrap), Enter select + save, Esc/Ctrl-D
//! cancel. Mirrors `skill_menu::state::handle_skill_key`'s `slot.take()`
//! pattern so `Option<ApMenu>` is never double-borrowed.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::{ApMode, Config};

use super::list::{AP_CHOICES, ap_mode_json, mode_index};

/// The `/ap` modal. `app.rs` holds `Option<ApMenu>`; while `Some`, all keys
/// route through [`handle_ap_key`].
pub struct ApMenu {
    /// Index into `AP_CHOICES` — the highlighted (cursor) row.
    pub selected: usize,
    /// The mode active when the menu opened — rendered with a `← 当前` mark.
    pub current: ApMode,
}

impl ApMenu {
    /// Pure constructor: the cursor starts on the config's current mode.
    pub fn new(config: &Config) -> Self {
        Self {
            selected: mode_index(config.autopilot.mode),
            current: config.autopilot.mode,
        }
    }
}

/// Outcome of a keystroke while the `/ap` modal is open.
pub enum ApOutcome {
    Idle,
    /// JSON merge-patch to persist via `Config::save`.
    Save(serde_json::Value),
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
    let Some(menu) = slot.take() else {
        return ApOutcome::Idle;
    };
    let (outcome, next) = match k.code {
        KeyCode::Up => (ApOutcome::Idle, Some(moved(menu, -1))),
        KeyCode::Down => (ApOutcome::Idle, Some(moved(menu, 1))),
        KeyCode::Enter => (
            ApOutcome::Save(ap_mode_json(AP_CHOICES[menu.selected].mode)),
            None,
        ),
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
        assert_eq!(review.current, ApMode::Review, "current mode is remembered for view");
    }

    /// ↑/↓ move with wrap-around and keep the menu open (Idle).
    #[test]
    fn up_down_move_with_wrap() {
        let mut slot = Some(ApMenu::new(&cfg_with(ApMode::Ap)));
        assert!(matches!(handle_ap_key(&mut slot, key(KeyCode::Down)), ApOutcome::Idle));
        assert_eq!(slot.as_ref().unwrap().selected, 2, "down: ap -> review");
        assert!(matches!(handle_ap_key(&mut slot, key(KeyCode::Down)), ApOutcome::Idle));
        assert_eq!(slot.as_ref().unwrap().selected, 0, "down from review wraps to off");
        assert!(matches!(handle_ap_key(&mut slot, key(KeyCode::Up)), ApOutcome::Idle));
        assert_eq!(slot.as_ref().unwrap().selected, 2, "up from off wraps to review");
        assert!(slot.is_some(), "movement keeps the menu open");
    }

    /// Enter returns the highlighted mode as a merge-patch and closes the menu.
    #[test]
    fn enter_saves_selected_mode_json_and_closes() {
        let mut slot = Some(ApMenu::new(&cfg_with(ApMode::Off)));
        handle_ap_key(&mut slot, key(KeyCode::Up)); // wrap onto review
        match handle_ap_key(&mut slot, key(KeyCode::Enter)) {
            ApOutcome::Save(json) => assert_eq!(
                json,
                serde_json::json!({ "autopilot": { "mode": "review" } })
            ),
            _ => panic!("expected Save"),
        }
        assert!(slot.is_none(), "Enter closes the menu");
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
        for code in [KeyCode::Enter, KeyCode::Esc, KeyCode::Up, KeyCode::Char('x')] {
            assert!(matches!(handle_ap_key(&mut slot, key(code)), ApOutcome::Idle));
        }
        assert!(slot.is_none());
    }

    /// Unmapped keys keep the menu open without moving the cursor.
    #[test]
    fn unmapped_key_is_idle_and_keeps_menu() {
        let mut slot = Some(ApMenu::new(&cfg_with(ApMode::Ap)));
        assert!(matches!(handle_ap_key(&mut slot, key(KeyCode::Char('x'))), ApOutcome::Idle));
        assert_eq!(slot.as_ref().unwrap().selected, 1, "cursor unmoved");
    }
}

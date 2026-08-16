use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::list::SkillList;

/// Active modal variant. `app.rs` holds `Option<SkillMenu>`.
pub enum SkillMenu {
    List(SkillList),
}

/// Outcome of a keystroke while the modal is open.
pub enum SkillOutcome {
    Idle,
    /// JSON merge-patch to persist via `Config::save`.
    Save(serde_json::Value),
    Cancel,
}

/// Handle one keystroke while the `/skill` modal is open. Mirrors
/// `mcp_menu::state::handle_mcp_key`: `slot.take()` moves ownership into
/// the per-mode handler so `Option<SkillMenu>` is never double-borrowed.
/// Ctrl-D always closes (hard-abort escape hatch for stuck modals).
pub fn handle_skill_key(slot: &mut Option<SkillMenu>, k: KeyEvent) -> SkillOutcome {
    if k.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(k.code, KeyCode::Char('d') | KeyCode::Char('\u{4}'))
    {
        *slot = None;
        return SkillOutcome::Cancel;
    }
    let Some(menu) = slot.take() else {
        return SkillOutcome::Idle;
    };
    let (outcome, next) = match menu {
        SkillMenu::List(list) => super::list::handle_key(list, k),
    };
    *slot = next;
    outcome
}

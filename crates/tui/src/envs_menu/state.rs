//! `/envs` modal state machine: variant enum + keystroke dispatcher.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::form::EnvNameForm;
use super::list::EnvsList;

/// Active modal variant. `app.rs` holds `Option<EnvsMenu>`.
pub enum EnvsMenu {
    List(EnvsList),
    Form(EnvNameForm),
}

/// Outcome of a keystroke while the modal is open. Unlike `/mcp` (JSON
/// merge-patches) envs mutations are side-effectful marker/dir operations on
/// `~/.opencoder/envs/` — the caller executes them via the core envs API.
pub enum EnvsOutcome {
    Idle,
    /// Activate env `name` (marker write + full config/client refresh).
    Activate(String),
    /// Clear the marker; base configuration becomes effective again.
    Deactivate,
    /// Create env `name`, optionally seeded from a base-chain capture.
    Create { name: String, capture: bool },
    /// Re-capture the base chain into env `name`.
    Recapture(String),
    /// Delete env `name` (clears the marker first when it is active).
    Delete(String),
    Cancel,
}

impl EnvsMenu {
    pub fn paste(&mut self, text: &str) {
        match self {
            EnvsMenu::Form(form) => form.paste_into(text),
            EnvsMenu::List(_) => {}
        }
    }
}

/// Handle one keystroke. Uses `slot.take()` to avoid double-borrow of
/// `Option<EnvsMenu>`: ownership moves into the per-mode handler.
pub fn handle_envs_key(slot: &mut Option<EnvsMenu>, k: KeyEvent) -> EnvsOutcome {
    if k.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(k.code, KeyCode::Char('d') | KeyCode::Char('\u{4}'))
    {
        *slot = None;
        return EnvsOutcome::Cancel;
    }
    let menu = match slot.take() {
        Some(m) => m,
        None => return EnvsOutcome::Idle,
    };
    let (outcome, next) = match menu {
        EnvsMenu::List(list) => super::list::handle_key(list, k),
        EnvsMenu::Form(form) => super::form::handle_key(form, k),
    };
    if let Some(m) = next {
        *slot = Some(m);
    }
    outcome
}

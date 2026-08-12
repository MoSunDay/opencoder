use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::form::McpForm;
use super::list::McpList;

/// Active modal variant. `app.rs` holds `Option<McpMenu>`.
pub enum McpMenu {
    List(McpList),
    Form(McpForm),
}

/// Outcome of a keystroke while the modal is open.
pub enum McpOutcome {
    Idle,
    /// JSON merge-patch to persist via `Config::save`.
    Save(serde_json::Value),
    Cancel,
}

impl McpMenu {
    pub fn paste(&mut self, text: &str) {
        match self {
            McpMenu::Form(form) => form.paste_into(text),
            McpMenu::List(_) => {}
        }
    }
}

/// Handle one keystroke. Uses `slot.take()` to avoid double-borrow of
/// `Option<McpMenu>`: ownership moves into the per-mode handler.
pub fn handle_mcp_key(slot: &mut Option<McpMenu>, k: KeyEvent) -> McpOutcome {
    if k.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(k.code, KeyCode::Char('d') | KeyCode::Char('\u{4}'))
    {
        *slot = None;
        return McpOutcome::Cancel;
    }
    let menu = match slot.take() {
        Some(m) => m,
        None => return McpOutcome::Idle,
    };
    let (outcome, next) = match menu {
        McpMenu::List(list) => super::list::handle_key(list, k),
        McpMenu::Form(form) => super::form::handle_key(form, k),
    };
    if let Some(m) = next {
        *slot = Some(m);
    }
    outcome
}

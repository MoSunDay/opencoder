//! Top-level state + dispatch for the `/config` and `/model` modals.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::config_form::ConfigForm;
use super::list::ProviderList;
use super::provider_form::ProviderForm;

/// The active modal variant. `app.rs` holds `Option<ModelMenu>`.
pub enum ModelMenu {
    /// `/config` — generation-parameter form.
    Config(ConfigForm),
    /// `/model` — provider list (select / edit / add / delete).
    List(ProviderList),
    /// `/model` — provider add/edit form.
    Form(ProviderForm),
}

/// Outcome of a keystroke while a modal is open.
pub enum ModelOutcome {
    Idle,
    /// Save with a pre-built JSON merge-patch for `Config::save`.
    Save(serde_json::Value),
    Cancel,
    Quit,
}

impl ModelMenu {
    /// Route pasted text to the focused field of the active form.
    /// `List` has no text fields and ignores paste.
    pub fn paste(&mut self, text: &str) {
        match self {
            ModelMenu::Config(form) => form.paste_into(text),
            ModelMenu::Form(form) => form.paste_into(text),
            ModelMenu::List(_) => {}
        }
    }
}

/// Handle one keystroke. Uses `slot.take()` to avoid double-borrow of the
/// `Option<ModelMenu>`: ownership moves into the per-mode handler, which
/// returns `(outcome, next_menu)`. If `next_menu` is `Some` the slot is
/// repopulated (idle or transition); `None` closes the modal.
pub fn handle_model_key(slot: &mut Option<ModelMenu>, k: KeyEvent) -> ModelOutcome {
    // Global Ctrl-D → Quit (works in any mode).
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        if matches!(k.code, KeyCode::Char('d') | KeyCode::Char('\u{4}')) {
            *slot = None;
            return ModelOutcome::Quit;
        }
        return ModelOutcome::Idle;
    }
    let menu = match slot.take() {
        Some(m) => m,
        None => return ModelOutcome::Idle,
    };
    let (outcome, next) = match menu {
        ModelMenu::Config(form) => super::config_form::handle_key(form, k),
        ModelMenu::List(list) => super::list::handle_key(list, k),
        ModelMenu::Form(form) => super::provider_form::handle_key(form, k),
    };
    if let Some(m) = next {
        *slot = Some(m);
    }
    outcome
}

/// `sk-****1234` style mask: keep the first 2 and last 4 chars when long
/// enough; otherwise show `****` to avoid leaking a short key verbatim.
pub fn mask_key(key: &str) -> String {
    let n = key.chars().count();
    if n == 0 {
        return "(unset)".to_string();
    }
    if n <= 6 {
        return "****".to_string();
    }
    let head: String = key.chars().take(2).collect();
    let tail: String = key.chars().skip(n.saturating_sub(4)).collect();
    format!("{head}****{tail}")
}

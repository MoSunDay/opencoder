use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{CliForm, CliList};

pub enum CliMenu {
    List(CliList),
    Form(CliForm),
}

pub enum CliOutcome {
    Idle,
    Save(serde_json::Value),
    Cancel,
}

impl CliMenu {
    pub fn paste(&mut self, text: &str) {
        if let Self::Form(form) = self {
            form.paste_into(text);
        }
    }
}

pub fn handle_cli_key(slot: &mut Option<CliMenu>, key: KeyEvent) -> CliOutcome {
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('d') | KeyCode::Char('\u{4}'))
    {
        *slot = None;
        return CliOutcome::Cancel;
    }
    let Some(menu) = slot.take() else {
        return CliOutcome::Idle;
    };
    let (outcome, next) = match menu {
        CliMenu::List(list) => super::list::handle_key(list, key),
        CliMenu::Form(form) => super::form::handle_key(form, key),
    };
    *slot = next;
    outcome
}

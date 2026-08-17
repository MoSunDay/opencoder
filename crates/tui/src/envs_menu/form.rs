//! `/envs` new-env name form: type a name, toggle capture, Enter to create.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use opencoder_core::validate_env_name;

use super::list::EnvsList;
use super::state::{EnvsMenu, EnvsOutcome};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvField {
    Name,
    Capture,
}

pub struct EnvNameForm {
    pub name: String,
    pub name_cursor: usize,
    /// Seed the new env from a base-chain capture (default on).
    pub capture: bool,
    /// Env names that already exist (duplicate guard; snapshot at open).
    pub existing: Vec<String>,
    pub field: EnvField,
}

impl EnvNameForm {
    pub fn new(existing: Vec<String>) -> Self {
        Self {
            name: String::new(),
            name_cursor: 0,
            capture: true,
            existing,
            field: EnvField::Name,
        }
    }

    /// Live validation feedback: `None` = submittable.
    pub fn validation_error(&self) -> Option<String> {
        if self.name.trim().is_empty() {
            return Some("名称不能为空".to_string());
        }
        if let Err(e) = validate_env_name(self.name.trim()) {
            return Some(e);
        }
        if self.existing.iter().any(|n| n == self.name.trim()) {
            return Some("同名 env 已存在".to_string());
        }
        None
    }
}

fn insert_at_cursor(s: &mut String, cursor: &mut usize, text: &str) {
    let byte_idx = s
        .char_indices()
        .nth(*cursor)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    s.insert_str(byte_idx, text);
    *cursor += text.chars().count();
}

fn backspace_at(s: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    *cursor -= 1;
    let byte_idx = s
        .char_indices()
        .nth(*cursor)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    s.remove(byte_idx);
}

impl EnvNameForm {
    pub fn paste_into(&mut self, text: &str) {
        if self.field == EnvField::Name {
            let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
            insert_at_cursor(&mut self.name, &mut self.name_cursor, &t);
        }
    }

    fn toggle_capture(&mut self) {
        self.capture = !self.capture;
    }
}

/// Handle one keystroke on the form. `Esc` returns to the list (fresh
/// discovery); `Enter` on Name submits when valid (closing the modal) or
/// stays put on a validation error.
pub fn handle_key(mut form: EnvNameForm, k: KeyEvent) -> (EnvsOutcome, Option<EnvsMenu>) {
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        // keep Ctrl+D close-to-modal reserved for the dispatcher; other chords
        // are swallowed except field clears
        if matches!(k.code, KeyCode::Char('l') | KeyCode::Char('u')) && form.field == EnvField::Name
        {
            form.name.clear();
            form.name_cursor = 0;
        }
        return (EnvsOutcome::Idle, Some(EnvsMenu::Form(form)));
    }
    match k.code {
        KeyCode::Esc => (
            EnvsOutcome::Idle,
            Some(EnvsMenu::List(EnvsList::discover())),
        ),
        KeyCode::Tab | KeyCode::Down => {
            form.field = match form.field {
                EnvField::Name => EnvField::Capture,
                EnvField::Capture => EnvField::Name,
            };
            (EnvsOutcome::Idle, Some(EnvsMenu::Form(form)))
        }
        KeyCode::Up => {
            form.field = EnvField::Name;
            (EnvsOutcome::Idle, Some(EnvsMenu::Form(form)))
        }
        KeyCode::Left => {
            if form.field == EnvField::Name && form.name_cursor > 0 {
                form.name_cursor -= 1;
            }
            (EnvsOutcome::Idle, Some(EnvsMenu::Form(form)))
        }
        KeyCode::Right => {
            if form.field == EnvField::Name && form.name_cursor < form.name.chars().count() {
                form.name_cursor += 1;
            }
            (EnvsOutcome::Idle, Some(EnvsMenu::Form(form)))
        }
        KeyCode::Enter | KeyCode::Char(' ') => match form.field {
            EnvField::Capture => {
                form.toggle_capture();
                (EnvsOutcome::Idle, Some(EnvsMenu::Form(form)))
            }
            EnvField::Name => {
                if k.code == KeyCode::Char(' ') {
                    insert_at_cursor(&mut form.name, &mut form.name_cursor, " ");
                    return (EnvsOutcome::Idle, Some(EnvsMenu::Form(form)));
                }
                match form.validation_error() {
                    None => (
                        EnvsOutcome::Create {
                            name: form.name.trim().to_string(),
                            capture: form.capture,
                        },
                        None,
                    ),
                    Some(_) => (EnvsOutcome::Idle, Some(EnvsMenu::Form(form))),
                }
            }
        },
        KeyCode::Backspace => {
            if form.field == EnvField::Name {
                backspace_at(&mut form.name, &mut form.name_cursor);
            }
            (EnvsOutcome::Idle, Some(EnvsMenu::Form(form)))
        }
        KeyCode::Char(c) => {
            if form.field == EnvField::Name {
                insert_at_cursor(&mut form.name, &mut form.name_cursor, &c.to_string());
            }
            (EnvsOutcome::Idle, Some(EnvsMenu::Form(form)))
        }
        _ => (EnvsOutcome::Idle, Some(EnvsMenu::Form(form))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, crossterm::event::KeyModifiers::NONE)
    }

    #[test]
    fn enter_submits_only_a_valid_non_duplicate_name() {
        let mut f = EnvNameForm::new(vec!["taken".into()]);
        for c in "work".chars() {
            let (_, next) = handle_key(f, key(KeyCode::Char(c)));
            let EnvsMenu::Form(g) = next.unwrap() else {
                panic!()
            };
            f = g;
        }
        assert!(f.validation_error().is_none());

        let (o, next) = handle_key(f, key(KeyCode::Enter));
        assert!(matches!(o, EnvsOutcome::Create { ref name, capture: true } if name == "work"));
        assert!(next.is_none(), "create closes the modal");
    }

    #[test]
    fn invalid_and_duplicate_names_block_submit() {
        let mut f = EnvNameForm::new(vec!["taken".into()]);
        f.name = "taken".into();
        f.name_cursor = 5;
        let (o, next) = handle_key(f, key(KeyCode::Enter));
        assert!(matches!(o, EnvsOutcome::Idle));
        assert!(matches!(next, Some(EnvsMenu::Form(_))));

        let mut f = EnvNameForm::new(vec![]);
        f.name = "a b".into();
        f.name_cursor = 3;
        let (o, _) = handle_key(f, key(KeyCode::Enter));
        assert!(matches!(o, EnvsOutcome::Idle), "space in name blocked");

        let f = EnvNameForm::new(vec![]);
        let (o, _) = handle_key(f, key(KeyCode::Enter));
        assert!(matches!(o, EnvsOutcome::Idle), "empty name blocked");
    }

    #[test]
    fn capture_toggle_and_space_typing() {
        let mut f = EnvNameForm::new(vec![]);
        f.field = EnvField::Capture;
        let (_, next) = handle_key(f, key(KeyCode::Char(' ')));
        let EnvsMenu::Form(g) = next.unwrap() else {
            panic!()
        };
        assert!(!g.capture, "space on capture toggles off");

        let mut f = EnvNameForm::new(vec![]);
        f.name = "ab".into();
        f.name_cursor = 1;
        let (_, next) = handle_key(f, key(KeyCode::Char(' ')));
        let EnvsMenu::Form(g) = next.unwrap() else {
            panic!()
        };
        assert_eq!(g.name, "a b", "space on name types a space");
    }

    #[test]
    fn esc_returns_to_list_and_paste_edits_name() {
        let f = EnvNameForm::new(vec![]);
        let (_, next) = handle_key(f, key(KeyCode::Esc));
        assert!(matches!(next, Some(EnvsMenu::List(_))));

        let mut f = EnvNameForm::new(vec![]);
        f.paste_into(" pas te ");
        assert_eq!(f.name, "paste", "paste strips whitespace");
    }
}

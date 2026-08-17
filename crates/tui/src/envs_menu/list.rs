//! `/envs` env list: activate (Enter), new (n), recapture (e), delete (d).

use crossterm::event::{KeyCode, KeyEvent};

use opencoder_core::{active_env, list_envs};

use super::form::EnvNameForm;
use super::state::{EnvsMenu, EnvsOutcome};

/// Row 0 is always `<base>` (no env); rows `1..` are the env names.
pub const BASE_ROW: usize = 0;

pub struct EnvsList {
    pub envs: Vec<String>,
    pub active: Option<String>,
    pub selected: usize,
    pub confirm_delete: Option<usize>,
}

impl EnvsList {
    /// Snapshot the envs root (pure filesystem reads; test-isolated via
    /// `scoped_config_home`).
    pub fn discover() -> Self {
        Self {
            envs: list_envs(),
            active: active_env(),
            selected: BASE_ROW,
            confirm_delete: None,
        }
    }

    /// The env name of the selected row, or `None` for the `<base>` row.
    pub fn selected_env(&self) -> Option<&str> {
        if self.selected == BASE_ROW {
            return None;
        }
        self.envs.get(self.selected - 1).map(String::as_str)
    }

    fn move_up(&mut self) {
        if self.selected > BASE_ROW {
            self.selected -= 1;
        }
    }

    fn move_down(&mut self) {
        if self.selected < self.envs.len() {
            self.selected += 1;
        }
    }
}

/// Handle one keystroke on the list. Returns `(outcome, next_menu)`;
/// `next_menu == None` closes the modal (terminal Activate/Deactivate/Create),
/// `Some(List(..))` keeps it open with fresh state after list mutations.
pub fn handle_key(mut list: EnvsList, k: KeyEvent) -> (EnvsOutcome, Option<EnvsMenu>) {
    // delete-confirmation sub-state owns keys first (mirrors /mcp)
    if let Some(idx) = list.confirm_delete {
        match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                let name = list.envs.get(idx.saturating_sub(1)).cloned();
                match name {
                    Some(n) => return (EnvsOutcome::Delete(n), None),
                    None => list.confirm_delete = None,
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => list.confirm_delete = None,
            _ => {}
        }
        return (EnvsOutcome::Idle, Some(EnvsMenu::List(list)));
    }

    match k.code {
        KeyCode::Esc => (EnvsOutcome::Cancel, None),
        KeyCode::Up => {
            list.move_up();
            (EnvsOutcome::Idle, Some(EnvsMenu::List(list)))
        }
        KeyCode::Down => {
            list.move_down();
            (EnvsOutcome::Idle, Some(EnvsMenu::List(list)))
        }
        KeyCode::Enter => match list.selected_env() {
            Some(name) => (EnvsOutcome::Activate(name.to_string()), None),
            None => match list.active.is_some() {
                true => (EnvsOutcome::Deactivate, None),
                false => (EnvsOutcome::Idle, Some(EnvsMenu::List(list))),
            },
        },
        KeyCode::Char('n') | KeyCode::Char('N') => {
            let existing = list.envs.clone();
            (
                EnvsOutcome::Idle,
                Some(EnvsMenu::Form(EnvNameForm::new(existing))),
            )
        }
        KeyCode::Char('e') | KeyCode::Char('E') => match list.selected_env() {
            Some(name) => (EnvsOutcome::Recapture(name.to_string()), None),
            None => (EnvsOutcome::Idle, Some(EnvsMenu::List(list))),
        },
        KeyCode::Char('d') | KeyCode::Char('D') => match list.selected_env() {
            Some(_) => {
                list.confirm_delete = Some(list.selected);
                (EnvsOutcome::Idle, Some(EnvsMenu::List(list)))
            }
            None => (EnvsOutcome::Idle, Some(EnvsMenu::List(list))),
        },
        _ => (EnvsOutcome::Idle, Some(EnvsMenu::List(list))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, crossterm::event::KeyModifiers::NONE)
    }

    fn list(envs: &[&str], active: Option<&str>, selected: usize) -> EnvsList {
        EnvsList {
            envs: envs.iter().map(|s| s.to_string()).collect(),
            active: active.map(String::from),
            selected,
            confirm_delete: None,
        }
    }

    #[test]
    fn enter_activates_env_and_base_deactivates() {
        let (o, next) = handle_key(list(&["a", "b"], Some("a"), 2), key(KeyCode::Enter));
        assert!(matches!(o, EnvsOutcome::Activate(ref n) if n == "b"));
        assert!(next.is_none(), "activating closes the modal");

        // base row with an active env -> deactivate
        let (o, _) = handle_key(list(&["a"], Some("a"), BASE_ROW), key(KeyCode::Enter));
        assert!(matches!(o, EnvsOutcome::Deactivate));

        // base row with NO active env -> idle no-op
        let (o, next) = handle_key(list(&["a"], None, BASE_ROW), key(KeyCode::Enter));
        assert!(matches!(o, EnvsOutcome::Idle));
        assert!(next.is_some());
    }

    #[test]
    fn navigation_clamps_between_base_and_last_env() {
        let (o, next) = handle_key(list(&["a", "b"], None, 0), key(KeyCode::Up));
        assert!(matches!(o, EnvsOutcome::Idle));
        let EnvsMenu::List(l) = next.unwrap() else {
            panic!()
        };
        assert_eq!(l.selected, 0, "up at base row clamps");

        let (_, next) = handle_key(list(&["a", "b"], None, 2), key(KeyCode::Down));
        let EnvsMenu::List(l) = next.unwrap() else {
            panic!()
        };
        assert_eq!(l.selected, 2, "down at last env clamps");
    }

    #[test]
    fn n_opens_form_with_existing_names() {
        let (_, next) = handle_key(list(&["a"], None, BASE_ROW), key(KeyCode::Char('n')));
        let EnvsMenu::Form(f) = next.unwrap() else {
            panic!()
        };
        assert_eq!(f.existing, vec!["a".to_string()]);
        assert!(f.capture, "capture defaults on");
    }

    #[test]
    fn e_and_d_require_an_env_row() {
        let (o, next) = handle_key(list(&["a"], None, BASE_ROW), key(KeyCode::Char('e')));
        assert!(matches!(o, EnvsOutcome::Idle));
        assert!(matches!(next, Some(EnvsMenu::List(_))));

        let (_, next) = handle_key(list(&["a"], None, 1), key(KeyCode::Char('d')));
        let EnvsMenu::List(l) = next.unwrap() else {
            panic!()
        };
        assert_eq!(l.confirm_delete, Some(1));

        // y confirms, n/Esc cancels
        let (o, _) = handle_key(
            EnvsList {
                envs: vec!["a".into()],
                active: None,
                selected: 1,
                confirm_delete: Some(1),
            },
            key(KeyCode::Char('y')),
        );
        assert!(matches!(o, EnvsOutcome::Delete(ref n) if n == "a"));
        let (_, next) = handle_key(
            EnvsList {
                envs: vec!["a".into()],
                active: None,
                selected: 1,
                confirm_delete: Some(1),
            },
            key(KeyCode::Esc),
        );
        let EnvsMenu::List(l) = next.unwrap() else {
            panic!()
        };
        assert!(l.confirm_delete.is_none());
    }

    #[test]
    fn esc_cancels() {
        let (o, next) = handle_key(list(&["a"], None, 0), key(KeyCode::Esc));
        assert!(matches!(o, EnvsOutcome::Cancel));
        assert!(next.is_none());
    }
}

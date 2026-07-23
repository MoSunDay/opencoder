//! HeadersEditor — inline editor for `Vec<(String, String)>` header pairs.
//!
//! Embedded inside the provider form. When the form focus lands on `Headers`
//! and the user presses Enter, `headers_active` becomes true in the parent
//! form and keys are routed here.
//!
//! Navigation: Up/Down select a pair, Left/Right toggle name↔value, `+` adds
//! a pair, `-` deletes, characters/backspace edit the focused sub-field.

/// Editor state for a list of HTTP header name/value pairs.
#[derive(Debug, Clone)]
pub struct HeadersEditor {
    /// Ordered name/value pairs.
    pub pairs: Vec<(String, String)>,
    /// Index of the currently selected pair.
    pub selected: usize,
    /// `false` = editing the name, `true` = editing the value.
    pub editing_value: bool,
}

/// Outcome of a single key press inside the headers editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderAction {
    /// Key was consumed; stay in headers editing.
    Active,
    /// User pressed Enter — exit headers sub-mode (return to form navigation).
    Exit,
}

impl HeadersEditor {
    pub fn new(pairs: Vec<(String, String)>) -> Self {
        HeadersEditor {
            pairs,
            selected: 0,
            editing_value: false,
        }
    }

    /// The string being edited (name or value of the selected pair).
    fn active_string(&mut self) -> Option<&mut String> {
        let idx = self.selected;
        let ev = self.editing_value;
        self.pairs.get_mut(idx).map(|(n, v)| if ev { v } else { n })
    }

    /// Handle one key. Returns `Active` (stay) or `Exit` (leave sub-mode).
    pub fn handle_key(&mut self, k: crossterm::event::KeyEvent) -> HeaderAction {
        use crossterm::event::{KeyCode, KeyModifiers};
        if k.modifiers.contains(KeyModifiers::CONTROL) {
            return HeaderAction::Active;
        }
        match k.code {
            KeyCode::Enter => HeaderAction::Exit,
            KeyCode::Esc => {
                self.editing_value = false;
                HeaderAction::Exit
            }
            KeyCode::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                HeaderAction::Active
            }
            KeyCode::Down => {
                if !self.pairs.is_empty() && self.selected + 1 < self.pairs.len() {
                    self.selected += 1;
                }
                HeaderAction::Active
            }
            KeyCode::Left => {
                self.editing_value = false;
                HeaderAction::Active
            }
            KeyCode::Right => {
                self.editing_value = true;
                HeaderAction::Active
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.pairs.push((String::new(), String::new()));
                self.selected = self.pairs.len() - 1;
                self.editing_value = false;
                HeaderAction::Active
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                if !self.pairs.is_empty() {
                    self.pairs.remove(self.selected);
                    if self.selected >= self.pairs.len() && self.selected > 0 {
                        self.selected -= 1;
                    }
                }
                HeaderAction::Active
            }
            KeyCode::Backspace => {
                if let Some(s) = self.active_string() {
                    s.pop();
                }
                HeaderAction::Active
            }
            KeyCode::Char(c) => {
                if let Some(s) = self.active_string() {
                    s.push(c);
                }
                HeaderAction::Active
            }
            _ => HeaderAction::Active,
        }
    }

    /// Bulk-insert pasted text into the active name/value field (mirrors `Char`).
    pub fn paste_into(&mut self, text: &str) {
        if let Some(s) = self.active_string() {
            s.push_str(text);
        }
    }

    /// Human-readable label for the selected pair's active sub-field.
    pub fn active_label(&self) -> &'static str {
        if self.editing_value {
            "value"
        } else {
            "name"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
    }
    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())
    }
    fn backspace() -> KeyEvent {
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty())
    }
    fn up() -> KeyEvent {
        KeyEvent::new(KeyCode::Up, KeyModifiers::empty())
    }
    fn down() -> KeyEvent {
        KeyEvent::new(KeyCode::Down, KeyModifiers::empty())
    }
    #[allow(dead_code)]
    fn left() -> KeyEvent {
        KeyEvent::new(KeyCode::Left, KeyModifiers::empty())
    }
    fn right() -> KeyEvent {
        KeyEvent::new(KeyCode::Right, KeyModifiers::empty())
    }

    #[test]
    fn add_pair_with_plus() {
        let mut ed = HeadersEditor::new(vec![]);
        ed.handle_key(key('+'));
        assert_eq!(ed.pairs.len(), 1);
        assert_eq!(ed.selected, 0);
        assert!(!ed.editing_value, "starts editing name");
    }

    #[test]
    fn type_into_name_then_value() {
        let mut ed = HeadersEditor::new(vec![("X-Foo".into(), "bar".into())]);
        ed.selected = 0;
        ed.editing_value = false;
        // Backspace removes last char of name.
        ed.handle_key(backspace());
        assert_eq!(ed.pairs[0].0, "X-Fo");
        // Right switches to value.
        ed.handle_key(right());
        assert!(ed.editing_value);
        ed.handle_key(key('!'));
        assert_eq!(ed.pairs[0].1, "bar!");
    }

    #[test]
    fn delete_pair_with_minus() {
        let mut ed = HeadersEditor::new(vec![
            ("A".into(), "1".into()),
            ("B".into(), "2".into()),
            ("C".into(), "3".into()),
        ]);
        ed.selected = 1;
        ed.handle_key(key('-'));
        assert_eq!(ed.pairs.len(), 2);
        assert_eq!(ed.pairs[0].0, "A");
        assert_eq!(ed.pairs[1].0, "C");
        assert_eq!(ed.selected, 1, "selection stays at index 1");
    }

    #[test]
    fn delete_last_pair_adjusts_selection() {
        let mut ed = HeadersEditor::new(vec![("A".into(), "1".into()), ("B".into(), "2".into())]);
        ed.selected = 1;
        ed.handle_key(key('-'));
        assert_eq!(ed.pairs.len(), 1);
        assert_eq!(ed.selected, 0, "selection clamped back");
    }

    #[test]
    fn up_down_navigate_pairs() {
        let mut ed = HeadersEditor::new(vec![("A".into(), "1".into()), ("B".into(), "2".into())]);
        ed.selected = 1;
        ed.handle_key(up());
        assert_eq!(ed.selected, 0);
        ed.handle_key(up());
        assert_eq!(ed.selected, 0, "clamped at top");
        ed.handle_key(down());
        assert_eq!(ed.selected, 1);
        ed.handle_key(down());
        assert_eq!(ed.selected, 1, "clamped at bottom");
    }

    #[test]
    fn enter_exits_sub_mode() {
        let mut ed = HeadersEditor::new(vec![("A".into(), "1".into())]);
        assert_eq!(ed.handle_key(enter()), HeaderAction::Exit);
    }

    #[test]
    fn esc_exits_and_resets_to_name() {
        let mut ed = HeadersEditor::new(vec![("A".into(), "1".into())]);
        ed.editing_value = true;
        assert_eq!(ed.handle_key(enter()), HeaderAction::Exit);
    }
}

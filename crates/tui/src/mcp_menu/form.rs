//! `/mcp` server add/edit form: name / enabled / command / args / url.
//! Save produces a JSON merge-patch for `Config::save`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::InjectionTarget;

use super::list::McpEntry;
use super::patch::save_mcp_json;
use super::state::{McpMenu, McpOutcome};
use crate::scope_dialog::{ScopeDialog, ScopeOutcome};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpField {
    Name,
    Enabled,
    InjectTo,
    Command,
    Args,
    Url,
}

impl McpField {
    fn next(self) -> Self {
        match self {
            McpField::Name => McpField::Enabled,
            McpField::Enabled => McpField::InjectTo,
            McpField::InjectTo => McpField::Command,
            McpField::Command => McpField::Args,
            McpField::Args => McpField::Url,
            McpField::Url => McpField::Name,
        }
    }
    fn prev(self) -> Self {
        match self {
            McpField::Name => McpField::Url,
            McpField::Enabled => McpField::Name,
            McpField::InjectTo => McpField::Enabled,
            McpField::Command => McpField::InjectTo,
            McpField::Args => McpField::Command,
            McpField::Url => McpField::Args,
        }
    }
}

pub struct McpForm {
    pub name: String,
    pub name_cursor: usize,
    pub original_name: Option<String>,
    pub enabled: bool,
    pub inject_to: InjectionTarget,
    pub command: String,
    pub command_cursor: usize,
    pub args: String,
    pub args_cursor: usize,
    pub url: String,
    pub url_cursor: usize,
    pub field: McpField,
    /// Open multi-select overlay for `inject_to` (Enter/Space on the field).
    pub scope_dialog: Option<ScopeDialog>,
}

impl McpForm {
    pub fn from_existing(entry: &McpEntry) -> Self {
        let args_joined = entry.args.join(" ");
        let command = entry.command.clone().unwrap_or_default();
        let url = entry.url.clone().unwrap_or_default();
        Self {
            name: entry.name.clone(),
            name_cursor: entry.name.chars().count(),
            original_name: Some(entry.name.clone()),
            enabled: entry.enabled,
            inject_to: entry.inject_to,
            command: command.clone(),
            command_cursor: command.chars().count(),
            args: args_joined.clone(),
            args_cursor: args_joined.chars().count(),
            url: url.clone(),
            url_cursor: url.chars().count(),
            field: McpField::Name,
            scope_dialog: None,
        }
    }

    pub fn new_blank() -> Self {
        Self {
            name: String::new(),
            name_cursor: 0,
            original_name: None,
            enabled: false,
            inject_to: InjectionTarget::parent_only(),
            command: String::new(),
            command_cursor: 0,
            args: String::new(),
            args_cursor: 0,
            url: String::new(),
            url_cursor: 0,
            field: McpField::Name,
            scope_dialog: None,
        }
    }

    pub fn paste_into(&mut self, text: &str) {
        if self.scope_dialog.is_some() {
            // Checkbox dialog has no text input: swallow the paste.
            return;
        }
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        match self.field {
            McpField::Name => insert_at_cursor(&mut self.name, &mut self.name_cursor, text),
            McpField::Command => {
                insert_at_cursor(&mut self.command, &mut self.command_cursor, text)
            }
            McpField::Args => {
                if !self.args.is_empty() {
                    insert_at_cursor(&mut self.args, &mut self.args_cursor, " ");
                }
                insert_at_cursor(&mut self.args, &mut self.args_cursor, text);
            }
            McpField::Url => insert_at_cursor(&mut self.url, &mut self.url_cursor, text),
            McpField::Enabled | McpField::InjectTo => {}
        }
    }

    fn is_text_field(&self) -> bool {
        !matches!(self.field, McpField::Enabled | McpField::InjectTo)
    }

    fn cursor_move_left(&mut self) {
        match self.field {
            McpField::Name => self.name_cursor = self.name_cursor.saturating_sub(1),
            McpField::Command => self.command_cursor = self.command_cursor.saturating_sub(1),
            McpField::Args => self.args_cursor = self.args_cursor.saturating_sub(1),
            McpField::Url => self.url_cursor = self.url_cursor.saturating_sub(1),
            McpField::Enabled | McpField::InjectTo => self.field = self.field.prev(),
        }
    }

    fn cursor_move_right(&mut self) {
        match self.field {
            McpField::Name => {
                let max = self.name.chars().count();
                if self.name_cursor < max {
                    self.name_cursor += 1;
                }
            }
            McpField::Command => {
                let max = self.command.chars().count();
                if self.command_cursor < max {
                    self.command_cursor += 1;
                }
            }
            McpField::Args => {
                let max = self.args.chars().count();
                if self.args_cursor < max {
                    self.args_cursor += 1;
                }
            }
            McpField::Url => {
                let max = self.url.chars().count();
                if self.url_cursor < max {
                    self.url_cursor += 1;
                }
            }
            McpField::Enabled | McpField::InjectTo => self.field = self.field.next(),
        }
    }

    fn backspace(&mut self) {
        match self.field {
            McpField::Name => backspace_at(&mut self.name, &mut self.name_cursor),
            McpField::Command => backspace_at(&mut self.command, &mut self.command_cursor),
            McpField::Args => backspace_at(&mut self.args, &mut self.args_cursor),
            McpField::Url => backspace_at(&mut self.url, &mut self.url_cursor),
            McpField::Enabled | McpField::InjectTo => {}
        }
    }

    fn type_char(&mut self, c: char) {
        match self.field {
            McpField::Name => {
                insert_at_cursor(&mut self.name, &mut self.name_cursor, &c.to_string())
            }
            McpField::Command => {
                insert_at_cursor(&mut self.command, &mut self.command_cursor, &c.to_string())
            }
            McpField::Args => {
                insert_at_cursor(&mut self.args, &mut self.args_cursor, &c.to_string())
            }
            McpField::Url => insert_at_cursor(&mut self.url, &mut self.url_cursor, &c.to_string()),
            McpField::Enabled | McpField::InjectTo => {}
        }
    }

    fn build_save(&self) -> Option<McpOutcome> {
        let name = self.name.trim();
        if name.is_empty() {
            return None;
        }
        let cmd = {
            let c = self.command.trim();
            if c.is_empty() {
                None
            } else {
                Some(c)
            }
        };
        let url = {
            let u = self.url.trim();
            if u.is_empty() {
                None
            } else {
                Some(u)
            }
        };
        let args: Vec<String> = self
            .args
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        Some(McpOutcome::Save(save_mcp_json(
            name,
            self.enabled,
            self.inject_to,
            cmd,
            &args,
            url,
            // Unedited names are filtered inside `save_mcp_json` (old != name).
            self.original_name.as_deref(),
        )))
    }
}

fn insert_at_cursor(buf: &mut String, cursor: &mut usize, text: &str) {
    let byte_idx = buf
        .char_indices()
        .nth(*cursor)
        .map(|(b, _)| b)
        .unwrap_or_else(|| buf.len());
    buf.insert_str(byte_idx, text);
    *cursor += text.chars().count();
}

fn backspace_at(buf: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let prev_byte = buf
        .char_indices()
        .nth(*cursor - 1)
        .map(|(b, _)| b)
        .unwrap_or(0);
    let char_len = buf[prev_byte..]
        .chars()
        .next()
        .map(|c| c.len_utf8())
        .unwrap_or(0);
    buf.drain(prev_byte..prev_byte + char_len);
    *cursor -= 1;
}

pub fn handle_key(mut form: McpForm, k: KeyEvent) -> (McpOutcome, Option<McpMenu>) {
    // The inject_to checkbox overlay owns the keyboard while open.
    if let Some(dialog) = form.scope_dialog.as_mut() {
        match dialog.handle_key(k) {
            ScopeOutcome::Confirm(target) => {
                form.scope_dialog = None;
                form.inject_to = target;
            }
            ScopeOutcome::Cancel => form.scope_dialog = None,
            ScopeOutcome::Idle => {}
        }
        return (McpOutcome::Idle, Some(McpMenu::Form(form)));
    }
    // Ctrl combos: Ctrl-U / Ctrl-L clear the focused text field; all other
    // Ctrl chords are swallowed so they never reach text input. (Mirrors the
    // /model provider_form dispatch.)
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        if matches!(
            k.code,
            KeyCode::Char('l')
                | KeyCode::Char('\u{c}')
                | KeyCode::Char('u')
                | KeyCode::Char('\u{15}')
        ) && form.is_text_field()
        {
            let (buf, cur) = current_buf_cursor(&mut form);
            buf.clear();
            *cur = 0;
        }
        return (McpOutcome::Idle, Some(McpMenu::Form(form)));
    }

    match k.code {
        KeyCode::Esc => return (McpOutcome::Cancel, None),
        KeyCode::Tab => form.field = form.field.next(),
        KeyCode::Up => form.field = form.field.prev(),
        KeyCode::Down => form.field = form.field.next(),
        KeyCode::Left => form.cursor_move_left(),
        KeyCode::Right => form.cursor_move_right(),
        KeyCode::Enter => {
            if form.field == McpField::Enabled {
                form.enabled = !form.enabled;
            } else if form.field == McpField::InjectTo {
                form.scope_dialog = Some(ScopeDialog::new(form.inject_to));
            } else if let Some(outcome) = form.build_save() {
                return (outcome, None);
            }
        }
        KeyCode::Backspace => form.backspace(),
        KeyCode::Char(c) => {
            if form.is_text_field() {
                form.type_char(c);
            } else if c == ' ' && form.field == McpField::Enabled {
                form.enabled = !form.enabled;
            } else if c == ' ' && form.field == McpField::InjectTo {
                form.scope_dialog = Some(ScopeDialog::new(form.inject_to));
            }
        }
        _ => {}
    }
    (McpOutcome::Idle, Some(McpMenu::Form(form)))
}

fn current_buf_cursor(form: &mut McpForm) -> (&mut String, &mut usize) {
    match form.field {
        McpField::Name => (&mut form.name, &mut form.name_cursor),
        McpField::Command => (&mut form.command, &mut form.command_cursor),
        McpField::Args => (&mut form.args, &mut form.args_cursor),
        McpField::Url => (&mut form.url, &mut form.url_cursor),
        McpField::Enabled | McpField::InjectTo => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::collections::HashMap;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn empty_name_does_not_save() {
        let form = McpForm::new_blank();
        // Press Enter on the Name field (which is empty).
        let (outcome, next) = handle_key(form, key(KeyCode::Enter));
        assert!(matches!(outcome, McpOutcome::Idle));
        assert!(matches!(next, Some(McpMenu::Form(_))));
    }

    #[test]
    fn typing_name_then_enter_saves() {
        let mut form = McpForm::new_blank();
        // Type "mysrv".
        for c in "mysrv".chars() {
            let (o, n) = handle_key(form, key(KeyCode::Char(c)));
            assert!(matches!(o, McpOutcome::Idle));
            form = match n {
                Some(McpMenu::Form(f)) => f,
                _ => panic!("expected Form"),
            };
        }
        assert_eq!(form.name, "mysrv");
        // Press Enter to save.
        let (outcome, next) = handle_key(form, key(KeyCode::Enter));
        match outcome {
            McpOutcome::Save(json) => {
                assert_eq!(json["mcp_servers"]["mysrv"]["enabled"], false);
            }
            _ => panic!("expected Save"),
        }
        assert!(next.is_none());
    }

    #[test]
    fn renaming_existing_server_nulls_old_key() {
        let entry = McpEntry {
            name: "a".to_string(),
            enabled: true,
            inject_to: InjectionTarget::parent_only(),
            command: Some("npx".to_string()),
            args: vec!["-y".to_string()],
            url: None,
            env: HashMap::new(),
        };
        let mut form = McpForm::from_existing(&entry);
        form.name = "b".to_string();
        form.name_cursor = 1;
        match form.build_save() {
            Some(McpOutcome::Save(json)) => {
                assert!(
                    json["mcp_servers"]["a"].is_null(),
                    "rename must null the old key"
                );
                assert!(
                    json["mcp_servers"]["b"].is_object(),
                    "rename must save under the new key"
                );
                assert_eq!(json["mcp_servers"]["b"]["command"], "npx");
                assert_eq!(json["mcp_servers"]["b"]["args"][0], "-y");
            }
            _ => panic!("expected Save"),
        }
    }

    #[test]
    fn tab_cycles_fields() {
        let mut form = McpForm::new_blank();
        assert_eq!(form.field, McpField::Name);
        form = match handle_key(form, key(KeyCode::Tab)).1 {
            Some(McpMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        assert_eq!(form.field, McpField::Enabled);
        form = match handle_key(form, key(KeyCode::Tab)).1 {
            Some(McpMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        assert_eq!(form.field, McpField::InjectTo);
        form = match handle_key(form, key(KeyCode::Tab)).1 {
            Some(McpMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        assert_eq!(form.field, McpField::Command);
    }

    #[test]
    fn space_toggles_enabled_field() {
        let mut form = McpForm::new_blank();
        // Move to Enabled field.
        form = match handle_key(form, key(KeyCode::Tab)).1 {
            Some(McpMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        assert_eq!(form.field, McpField::Enabled);
        assert!(!form.enabled);
        // Press space to toggle.
        form = match handle_key(form, key(KeyCode::Char(' '))).1 {
            Some(McpMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        assert!(form.enabled);
    }

    #[test]
    fn enter_on_inject_to_opens_and_confirms_scope_dialog() {
        let mut form = McpForm::new_blank();
        // Name -> Enabled -> InjectTo
        for _ in 0..2 {
            form = match handle_key(form, key(KeyCode::Tab)).1 {
                Some(McpMenu::Form(f)) => f,
                _ => panic!("expected Form"),
            };
        }
        assert_eq!(form.field, McpField::InjectTo);
        form = match handle_key(form, key(KeyCode::Enter)).1 {
            Some(McpMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        assert!(form.scope_dialog.is_some());
        // uncheck parent (row 0), check explore (row 1), confirm
        form = match handle_key(form, key(KeyCode::Char(' '))).1 {
            Some(McpMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        form = match handle_key(form, key(KeyCode::Down)).1 {
            Some(McpMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        form = match handle_key(form, key(KeyCode::Char(' '))).1 {
            Some(McpMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        form = match handle_key(form, key(KeyCode::Enter)).1 {
            Some(McpMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        assert!(form.scope_dialog.is_none());
        assert!(!form.inject_to.parent);
        assert!(form.inject_to.explore);
        assert!(!form.inject_to.build);
    }

    #[test]
    fn paste_is_swallowed_while_scope_dialog_open() {
        let mut form = McpForm::new_blank();
        form.field = McpField::InjectTo;
        form = match handle_key(form, key(KeyCode::Enter)).1 {
            Some(McpMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        form.paste_into("ignored");
        assert_eq!(form.inject_to, InjectionTarget::parent_only());
    }

    #[test]
    fn escape_cancels_form() {
        let form = McpForm::new_blank();
        let (outcome, next) = handle_key(form, key(KeyCode::Esc));
        assert!(matches!(outcome, McpOutcome::Cancel));
        assert!(next.is_none());
    }
}

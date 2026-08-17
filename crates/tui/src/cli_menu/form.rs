use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::InjectionTarget;

use super::content_dialog::{ContentDialog, ContentOutcome};
use super::list::{save_json, CliEntry};
use super::{CliMenu, CliOutcome};
use crate::scope_dialog::{ScopeDialog, ScopeOutcome};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CliField {
    Name,
    Enabled,
    InjectTo,
    Content,
}

impl CliField {
    fn next(self) -> Self {
        match self {
            Self::Name => Self::Enabled,
            Self::Enabled => Self::InjectTo,
            Self::InjectTo => Self::Content,
            Self::Content => Self::Name,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::Name => Self::Content,
            Self::Enabled => Self::Name,
            Self::InjectTo => Self::Enabled,
            Self::Content => Self::InjectTo,
        }
    }
}

pub struct CliForm {
    pub name: String,
    pub name_cursor: usize,
    pub original_name: Option<String>,
    pub enabled: bool,
    pub inject_to: InjectionTarget,
    pub content: String,
    pub content_cursor: usize,
    pub field: CliField,
    /// Open multi-select overlay for `inject_to` (Enter/Space on the field).
    pub scope_dialog: Option<ScopeDialog>,
    /// Open multi-line editor overlay for `content` (Enter on the field).
    pub content_dialog: Option<ContentDialog>,
}

impl CliForm {
    pub fn new_blank() -> Self {
        Self {
            name: String::new(),
            name_cursor: 0,
            original_name: None,
            enabled: false,
            inject_to: InjectionTarget::parent_only(),
            content: String::new(),
            content_cursor: 0,
            field: CliField::Name,
            scope_dialog: None,
            content_dialog: None,
        }
    }

    pub fn from_existing(entry: &CliEntry) -> Self {
        Self {
            name: entry.name.clone(),
            name_cursor: entry.name.chars().count(),
            original_name: Some(entry.name.clone()),
            enabled: entry.enabled,
            inject_to: entry.inject_to,
            content: entry.content.clone(),
            content_cursor: entry.content.chars().count(),
            field: CliField::Name,
            scope_dialog: None,
            content_dialog: None,
        }
    }

    pub fn paste_into(&mut self, text: &str) {
        if let Some(dialog) = self.content_dialog.as_mut() {
            dialog.insert_text(text);
            return;
        }
        if self.scope_dialog.is_some() {
            // Checkbox dialog has no text input: swallow the paste.
            return;
        }
        match self.field {
            CliField::Name => insert(&mut self.name, &mut self.name_cursor, text.trim()),
            CliField::Content => insert(&mut self.content, &mut self.content_cursor, text),
            CliField::Enabled | CliField::InjectTo => {}
        }
    }

    pub fn display_content(&self) -> String {
        self.content
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn open_scope_dialog(&mut self) {
        self.scope_dialog = Some(ScopeDialog::new(self.inject_to));
    }

    fn open_content_dialog(&mut self) {
        let dialog = ContentDialog::new(self.content.clone(), self.content_cursor);
        self.content_dialog = Some(dialog);
    }

    fn save(&self) -> Option<CliOutcome> {
        let name = self.name.trim();
        (!name.is_empty()).then(|| {
            CliOutcome::Save(save_json(
                name,
                self.enabled,
                self.inject_to,
                &self.content,
                // Unedited names are filtered inside `save_json` (old != name).
                self.original_name.as_deref(),
            ))
        })
    }
}

fn insert(buf: &mut String, cursor: &mut usize, text: &str) {
    let byte = buf
        .char_indices()
        .nth(*cursor)
        .map(|(i, _)| i)
        .unwrap_or(buf.len());
    buf.insert_str(byte, text);
    *cursor += text.chars().count();
}

fn backspace(buf: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let start = buf
        .char_indices()
        .nth(*cursor - 1)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let end = buf
        .char_indices()
        .nth(*cursor)
        .map(|(i, _)| i)
        .unwrap_or(buf.len());
    buf.drain(start..end);
    *cursor -= 1;
}

fn text_parts(form: &mut CliForm) -> Option<(&mut String, &mut usize)> {
    match form.field {
        CliField::Name => Some((&mut form.name, &mut form.name_cursor)),
        CliField::Content => Some((&mut form.content, &mut form.content_cursor)),
        CliField::Enabled | CliField::InjectTo => None,
    }
}

pub fn handle_key(mut form: CliForm, key: KeyEvent) -> (CliOutcome, Option<CliMenu>) {
    // Overlays own the keyboard while open.
    if let Some(dialog) = form.content_dialog.as_mut() {
        match dialog.handle_key(key) {
            ContentOutcome::Apply => {
                if let Some(dialog) = form.content_dialog.take() {
                    form.content = dialog.text;
                    form.content_cursor = dialog.cursor;
                }
            }
            ContentOutcome::Cancel => form.content_dialog = None,
            ContentOutcome::Idle => {}
        }
        return (CliOutcome::Idle, Some(CliMenu::Form(form)));
    }
    if let Some(dialog) = form.scope_dialog.as_mut() {
        match dialog.handle_key(key) {
            ScopeOutcome::Confirm(target) => {
                form.scope_dialog = None;
                form.inject_to = target;
            }
            ScopeOutcome::Cancel => form.scope_dialog = None,
            ScopeOutcome::Idle => {}
        }
        return (CliOutcome::Idle, Some(CliMenu::Form(form)));
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if matches!(
            key.code,
            KeyCode::Char('u')
                | KeyCode::Char('\u{15}')
                | KeyCode::Char('l')
                | KeyCode::Char('\u{c}')
        ) {
            if let Some((buf, cursor)) = text_parts(&mut form) {
                buf.clear();
                *cursor = 0;
            }
        }
        return (CliOutcome::Idle, Some(CliMenu::Form(form)));
    }
    match key.code {
        KeyCode::Esc => return (CliOutcome::Cancel, None),
        KeyCode::Tab | KeyCode::Down => form.field = form.field.next(),
        KeyCode::Up => form.field = form.field.prev(),
        KeyCode::Left => {
            if let Some((_, cursor)) = text_parts(&mut form) {
                *cursor = cursor.saturating_sub(1);
            }
        }
        KeyCode::Right => {
            if let Some((buf, cursor)) = text_parts(&mut form) {
                *cursor = (*cursor + 1).min(buf.chars().count());
            }
        }
        KeyCode::Enter => match form.field {
            CliField::Enabled => form.enabled = !form.enabled,
            CliField::InjectTo => form.open_scope_dialog(),
            CliField::Content => form.open_content_dialog(),
            CliField::Name => {
                if let Some(outcome) = form.save() {
                    return (outcome, None);
                }
            }
        },
        KeyCode::Backspace => {
            if let Some((buf, cursor)) = text_parts(&mut form) {
                backspace(buf, cursor);
            }
        }
        KeyCode::Char(' ') if form.field == CliField::Enabled => form.enabled = !form.enabled,
        KeyCode::Char(' ') if form.field == CliField::InjectTo => form.open_scope_dialog(),
        KeyCode::Char(ch) => {
            if let Some((buf, cursor)) = text_parts(&mut form) {
                insert(buf, cursor, &ch.to_string());
            }
        }
        _ => {}
    }
    (CliOutcome::Idle, Some(CliMenu::Form(form)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn form_on(field: CliField) -> CliForm {
        let mut form = CliForm::new_blank();
        form.field = field;
        form
    }

    #[test]
    fn enter_on_inject_to_opens_scope_dialog() {
        let form = form_on(CliField::InjectTo);
        let (_, next) = handle_key(form, key(KeyCode::Enter));
        let form = match next {
            Some(CliMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        assert!(form.scope_dialog.is_some());
    }

    #[test]
    fn scope_dialog_flow_confirms_selection() {
        // Enter opens; space unchecks parent; down+space checks explore; Enter applies.
        let form = form_on(CliField::InjectTo);
        let mut form = match handle_key(form, key(KeyCode::Enter)).1 {
            Some(CliMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        for k in [
            key(KeyCode::Char(' ')),
            key(KeyCode::Down),
            key(KeyCode::Char(' ')),
            key(KeyCode::Enter),
        ] {
            form = match handle_key(form, k).1 {
                Some(CliMenu::Form(f)) => f,
                _ => panic!("expected Form"),
            };
        }
        assert!(form.scope_dialog.is_none(), "confirm closes the dialog");
        assert!(!form.inject_to.parent);
        assert!(form.inject_to.explore);
        assert!(!form.inject_to.build);
    }

    #[test]
    fn scope_dialog_escape_discards_changes() {
        let form = form_on(CliField::InjectTo);
        let mut form = match handle_key(form, key(KeyCode::Enter)).1 {
            Some(CliMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        form = match handle_key(form, key(KeyCode::Char(' '))).1 {
            Some(CliMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        form = match handle_key(form, key(KeyCode::Esc)).1 {
            Some(CliMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        assert!(form.scope_dialog.is_none());
        assert_eq!(form.inject_to, InjectionTarget::parent_only());
    }

    #[test]
    fn enter_on_content_opens_multiline_dialog() {
        let form = form_on(CliField::Content);
        let (_, next) = handle_key(form, key(KeyCode::Enter));
        let form = match next {
            Some(CliMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        assert!(form.content_dialog.is_some());
    }

    #[test]
    fn content_dialog_ctrl_s_writes_back_text() {
        let mut form = form_on(CliField::Content);
        form.content = "base".into();
        form.content_cursor = 4;
        let mut form = match handle_key(form, key(KeyCode::Enter)).1 {
            Some(CliMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        // type a newline + text inside the dialog
        form.paste_into("more\nlines");
        let form = match handle_key(
            form,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
        )
        .1
        {
            Some(CliMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        assert!(form.content_dialog.is_none(), "apply closes the dialog");
        assert_eq!(form.content, "basemore\nlines");
        assert_eq!(
            form.display_content(),
            "basemore lines",
            "form preview stays single-line"
        );
    }

    #[test]
    fn content_dialog_esc_keeps_original_content() {
        let mut form = form_on(CliField::Content);
        form.content = "base".into();
        let mut form = match handle_key(form, key(KeyCode::Enter)).1 {
            Some(CliMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        form.paste_into("junk");
        let form = match handle_key(form, key(KeyCode::Esc)).1 {
            Some(CliMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        assert!(form.content_dialog.is_none());
        assert_eq!(form.content, "base");
    }

    #[test]
    fn paste_while_scope_dialog_open_is_swallowed() {
        let form = form_on(CliField::InjectTo);
        let mut form = match handle_key(form, key(KeyCode::Enter)).1 {
            Some(CliMenu::Form(f)) => f,
            _ => panic!("expected Form"),
        };
        form.paste_into("ignored");
        assert_eq!(form.inject_to, InjectionTarget::parent_only());
    }

    #[test]
    fn enter_on_name_saves() {
        let mut form = form_on(CliField::Name);
        form.name = "mycli".into();
        let (outcome, next) = handle_key(form, key(KeyCode::Enter));
        match outcome {
            CliOutcome::Save(json) => {
                assert_eq!(
                    json["cli"]["mycli"]["inject_to"],
                    serde_json::json!(["parent"])
                );
            }
            _ => panic!("expected Save"),
        }
        assert!(next.is_none());
    }

    #[test]
    fn renaming_existing_entry_nulls_old_key() {
        let entry = CliEntry {
            name: "a".into(),
            enabled: true,
            inject_to: InjectionTarget::parent_only(),
            content: "body".into(),
        };
        let mut form = CliForm::from_existing(&entry);
        form.name = "b".into();
        form.name_cursor = 1;
        let (outcome, _) = handle_key(form, key(KeyCode::Enter));
        match outcome {
            CliOutcome::Save(json) => {
                assert!(json["cli"]["a"].is_null(), "rename must null the old key");
                assert!(
                    json["cli"]["b"].is_object(),
                    "rename must save under the new key"
                );
                assert_eq!(json["cli"]["b"]["content"], "body");
            }
            _ => panic!("expected Save"),
        }
    }
}

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::InjectionTarget;

use super::list::{save_json, CliEntry};
use super::{CliMenu, CliOutcome};

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
}

impl CliForm {
    pub fn new_blank() -> Self {
        Self {
            name: String::new(),
            name_cursor: 0,
            original_name: None,
            enabled: false,
            inject_to: InjectionTarget::Parent,
            content: String::new(),
            content_cursor: 0,
            field: CliField::Name,
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
        }
    }

    pub fn paste_into(&mut self, text: &str) {
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

    fn save(&self) -> Option<CliOutcome> {
        let name = self.name.trim();
        (!name.is_empty())
            .then(|| CliOutcome::Save(save_json(name, self.enabled, self.inject_to, &self.content)))
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
        KeyCode::Enter => {
            if form.field == CliField::Enabled {
                form.enabled = !form.enabled;
            } else if form.field == CliField::InjectTo {
                form.inject_to = form.inject_to.next();
            } else if let Some(outcome) = form.save() {
                return (outcome, None);
            }
        }
        KeyCode::Backspace => {
            if let Some((buf, cursor)) = text_parts(&mut form) {
                backspace(buf, cursor);
            }
        }
        KeyCode::Char(' ') if form.field == CliField::Enabled => form.enabled = !form.enabled,
        KeyCode::Char(' ') if form.field == CliField::InjectTo => {
            form.inject_to = form.inject_to.next()
        }
        KeyCode::Char(ch) => {
            if let Some((buf, cursor)) = text_parts(&mut form) {
                insert(buf, cursor, &ch.to_string());
            }
        }
        _ => {}
    }
    (CliOutcome::Idle, Some(CliMenu::Form(form)))
}

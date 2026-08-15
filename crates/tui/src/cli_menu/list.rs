use crossterm::event::{KeyCode, KeyEvent};
use opencoder_core::{config::CliConfig, Config, InjectionTarget};
use serde_json::{json, Value};

use super::{CliForm, CliMenu, CliOutcome};

#[derive(Clone)]
pub struct CliEntry {
    pub name: String,
    pub enabled: bool,
    pub inject_to: InjectionTarget,
    pub content: String,
}

impl CliEntry {
    fn from_config(name: &str, cfg: &CliConfig) -> Self {
        Self {
            name: name.to_string(),
            enabled: cfg.enabled,
            inject_to: cfg.inject_to,
            content: cfg.content.clone(),
        }
    }

    pub fn summary(&self) -> String {
        let compact = self
            .content
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if compact.is_empty() {
            "(empty content)".to_string()
        } else {
            compact
        }
    }
}

pub struct CliList {
    pub entries: Vec<CliEntry>,
    pub selected: usize,
    pub confirm_delete: Option<usize>,
}

impl CliList {
    pub fn new(config: &Config) -> Self {
        let mut entries: Vec<_> = config
            .cli
            .iter()
            .map(|(name, cfg)| CliEntry::from_config(name, cfg))
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Self {
            entries,
            selected: 0,
            confirm_delete: None,
        }
    }
}

pub fn save_json(name: &str, enabled: bool, inject_to: InjectionTarget, content: &str) -> Value {
    json!({ "cli": { name: { "enabled": enabled, "inject_to": inject_to, "content": content } } })
}

fn toggle_json(name: &str, enabled: bool) -> Value {
    json!({ "cli": { name: { "enabled": enabled } } })
}

fn delete_json(name: &str) -> Value {
    json!({ "cli": { name: Value::Null } })
}

pub fn handle_key(mut list: CliList, key: KeyEvent) -> (CliOutcome, Option<CliMenu>) {
    if let Some(index) = list.confirm_delete {
        return match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                let name = list
                    .entries
                    .get(index)
                    .map(|e| e.name.as_str())
                    .unwrap_or("");
                (CliOutcome::Save(delete_json(name)), None)
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                list.confirm_delete = None;
                (CliOutcome::Idle, Some(CliMenu::List(list)))
            }
            _ => (CliOutcome::Idle, Some(CliMenu::List(list))),
        };
    }
    if list.entries.is_empty() {
        return match key.code {
            KeyCode::Char('n') => (CliOutcome::Idle, Some(CliMenu::Form(CliForm::new_blank()))),
            KeyCode::Enter | KeyCode::Esc => (CliOutcome::Cancel, None),
            _ => (CliOutcome::Idle, Some(CliMenu::List(list))),
        };
    }
    match key.code {
        KeyCode::Enter | KeyCode::Esc => (CliOutcome::Cancel, None),
        KeyCode::Up => {
            list.selected = list.selected.saturating_sub(1);
            (CliOutcome::Idle, Some(CliMenu::List(list)))
        }
        KeyCode::Down => {
            list.selected = (list.selected + 1).min(list.entries.len() - 1);
            (CliOutcome::Idle, Some(CliMenu::List(list)))
        }
        KeyCode::Left | KeyCode::Right => {
            let entry = &mut list.entries[list.selected];
            entry.enabled = !entry.enabled;
            let patch = toggle_json(&entry.name, entry.enabled);
            (CliOutcome::Save(patch), Some(CliMenu::List(list)))
        }
        KeyCode::Char('e') => {
            let form = CliForm::from_existing(&list.entries[list.selected]);
            (CliOutcome::Idle, Some(CliMenu::Form(form)))
        }
        KeyCode::Char('n') => (CliOutcome::Idle, Some(CliMenu::Form(CliForm::new_blank()))),
        KeyCode::Char('d') => {
            list.confirm_delete = Some(list.selected);
            (CliOutcome::Idle, Some(CliMenu::List(list)))
        }
        _ => (CliOutcome::Idle, Some(CliMenu::List(list))),
    }
}

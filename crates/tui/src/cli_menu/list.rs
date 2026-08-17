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

/// Build a save/upsert merge-patch for one CLI entry.
///
/// `renamed_from` carries the entry's pre-edit key (edit mode only). When it
/// differs from `name`, the domain object also sets the old key to null —
/// merge-patch semantics delete nulled keys, so without this a rename would
/// leave both `old` and `name` in cli.json and the content would be injected
/// twice. The `old == name` filter lives here (an unconditional null would
/// self-delete the just-saved entry), so callers may pass `original_name`
/// as-is.
pub fn save_json(
    name: &str,
    enabled: bool,
    inject_to: InjectionTarget,
    content: &str,
    renamed_from: Option<&str>,
) -> Value {
    let mut entry = serde_json::Map::new();
    entry.insert("enabled".to_string(), json!(enabled));
    entry.insert("inject_to".to_string(), json!(inject_to));
    entry.insert("content".to_string(), json!(content));
    // Built via an explicit Map so the old (null) and new keys provably
    // coexist in one object — a nested `json!` with a variable key makes
    // that invariant too easy to break silently.
    let mut cli = serde_json::Map::new();
    if let Some(old) = renamed_from.filter(|old| *old != name) {
        cli.insert(old.to_string(), Value::Null);
    }
    cli.insert(name.to_string(), Value::Object(entry));
    json!({ "cli": Value::Object(cli) })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_nulls_old_key_on_rename() {
        let v = save_json("b", true, InjectionTarget::parent_only(), "body", Some("a"));
        assert!(v["cli"]["a"].is_null(), "old key must be nulled");
        assert!(v["cli"]["b"].is_object(), "new key must carry the entry");
        assert_eq!(v["cli"]["b"]["enabled"], true);
        assert_eq!(v["cli"]["b"]["content"], "body");
    }

    #[test]
    fn save_keeps_entry_when_name_unchanged() {
        let v = save_json("a", true, InjectionTarget::parent_only(), "body", Some("a"));
        assert!(
            v["cli"]["a"].is_object(),
            "unchanged name must not self-delete"
        );
        assert_eq!(v["cli"]["a"]["enabled"], true);
    }

    #[test]
    fn save_without_rename_writes_single_key() {
        let v = save_json("a", false, InjectionTarget::parent_only(), "body", None);
        assert!(v["cli"]["a"].is_object());
        assert_eq!(
            v["cli"].as_object().map(|o| o.len()),
            Some(1),
            "no stray null keys for a fresh entry"
        );
    }
}

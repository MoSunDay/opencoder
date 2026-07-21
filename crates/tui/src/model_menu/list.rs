//! `/model` provider list: select (Enter), edit (e), add (n), delete (d).

use crossterm::event::{KeyCode, KeyEvent};
use opencoder_core::Config;

use super::patch::{delete_provider_json, switch_provider_json};
use super::provider_form::ProviderForm;
use super::state::{ModelMenu, ModelOutcome};

/// One row in the provider list.
#[derive(Debug, Clone)]
pub struct ProviderEntry {
    pub name: String,
    pub base_url: String,
    pub model_id: String,
    pub api_key: String,
    pub headers: Vec<(String, String)>,
    pub active: bool,
}

pub struct ProviderList {
    pub entries: Vec<ProviderEntry>,
    pub selected: usize,
    /// `Some(i)` = confirming deletion of entry `i`.
    pub confirm_delete: Option<usize>,
    pub default_base_url: String,
}

impl ProviderList {
    pub fn new(config: &Config) -> Self {
        let active = config.provider_id();
        let mut entries: Vec<ProviderEntry> = config
            .providers
            .iter()
            .map(|(name, p)| ProviderEntry {
                name: name.clone(),
                base_url: p.base_url.clone(),
                model_id: p
                    .model
                    .clone()
                    .unwrap_or_else(|| config.model_id().to_string()),
                api_key: p.api_key.clone().unwrap_or_default(),
                headers: p.headers.iter().map(|h| (h.name.clone(), h.value.clone())).collect(),
                active: name == active,
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let selected = entries.iter().position(|e| e.active).unwrap_or(0);
        ProviderList {
            entries,
            selected,
            confirm_delete: None,
            default_base_url: config.base_url_for(config.provider_id()),
        }
    }
}

/// Handle a key in provider-list mode.
pub fn handle_key(mut list: ProviderList, k: KeyEvent) -> (ModelOutcome, Option<ModelMenu>) {
    // Delete-confirmation sub-state takes priority.
    if let Some(idx) = list.confirm_delete {
        match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                let name = list.entries.get(idx).map(|e| e.name.clone());
                list.confirm_delete = None;
                if let Some(n) = name {
                    return (ModelOutcome::Save(delete_provider_json(&n)), None);
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                list.confirm_delete = None;
            }
            _ => {}
        }
        return (ModelOutcome::Idle, Some(ModelMenu::List(list)));
    }

    match k.code {
        KeyCode::Esc => (ModelOutcome::Cancel, None),
        KeyCode::Up => {
            if list.selected > 0 {
                list.selected -= 1;
            }
            (ModelOutcome::Idle, Some(ModelMenu::List(list)))
        }
        KeyCode::Down => {
            let n = list.entries.len();
            if n > 0 && list.selected + 1 < n {
                list.selected += 1;
            }
            (ModelOutcome::Idle, Some(ModelMenu::List(list)))
        }
        KeyCode::Enter => {
            if let Some(entry) = list.entries.get(list.selected) {
                let json = switch_provider_json(&entry.name, &entry.model_id);
                (ModelOutcome::Save(json), None)
            } else {
                (ModelOutcome::Idle, Some(ModelMenu::List(list)))
            }
        }
        KeyCode::Char('e') => {
            if let Some(entry) = list.entries.get(list.selected).cloned() {
                let form = ProviderForm::from_existing(
                    &entry.name,
                    &entry.base_url,
                    &entry.model_id,
                    &entry.api_key,
                    entry.headers,
                );
                (ModelOutcome::Idle, Some(ModelMenu::Form(form)))
            } else {
                (ModelOutcome::Idle, Some(ModelMenu::List(list)))
            }
        }
        KeyCode::Char('n') => {
            // Need config for defaults — but we don't have it here. Create a
            // minimal form; the caller (app.rs) has the config but the list
            // was already constructed from it. Use the first entry's base_url
            // as a hint, or a generic default.
            let default_base = list.default_base_url.clone();
            let form = ProviderForm {
                name: String::new(),
                name_readonly: false,
                model_id: String::new(),
                base_url: default_base,
                api_key_input: String::new(),
                api_key_original: String::new(),
                api_key_edited: false,
                headers: super::headers::HeadersEditor::new(Vec::new()),
                headers_active: false,
                focus: super::provider_form::ProviderField::Name,
                error: None,
            };
            (ModelOutcome::Idle, Some(ModelMenu::Form(form)))
        }
        KeyCode::Char('d') => {
            if !list.entries.is_empty() {
                list.confirm_delete = Some(list.selected);
            }
            (ModelOutcome::Idle, Some(ModelMenu::List(list)))
        }
        _ => (ModelOutcome::Idle, Some(ModelMenu::List(list))),
    }
}

//! `/mcp` server list: toggle (Enter), edit (e), add (n), delete (d).

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent};
use opencoder_core::config::McpServerConfig;
use opencoder_core::Config;

use super::form::McpForm;
use super::patch::{delete_mcp_json, toggle_mcp_json};
use super::state::{McpMenu, McpOutcome};

/// A snapshot of one MCP server for display and selection.
#[derive(Clone)]
pub struct McpEntry {
    pub name: String,
    pub enabled: bool,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    #[allow(dead_code)]
    pub env: HashMap<String, String>,
}

impl McpEntry {
    pub fn from_config(name: &str, cfg: &McpServerConfig) -> Self {
        Self {
            name: name.to_string(),
            enabled: cfg.enabled,
            command: cfg.command.clone(),
            args: cfg.args.clone(),
            url: cfg.url.clone(),
            env: cfg.env.clone(),
        }
    }

    /// Human-readable transport description.
    pub fn transport_label(&self) -> String {
        if let Some(cmd) = &self.command {
            let mut s = cmd.clone();
            for a in &self.args {
                s.push(' ');
                s.push_str(a);
            }
            format!("stdio: {}", s)
        } else if let Some(url) = &self.url {
            format!("sse: {}", url)
        } else {
            "(no transport)".to_string()
        }
    }
}

pub struct McpList {
    pub entries: Vec<McpEntry>,
    pub selected: usize,
    pub confirm_delete: Option<usize>,
}

impl McpList {
    pub fn new(config: &Config) -> Self {
        let mut names: Vec<String> = config.mcp_servers.keys().cloned().collect();
        names.sort();
        let entries = names
            .iter()
            .map(|n| McpEntry::from_config(n, &config.mcp_servers[n]))
            .collect();
        Self {
            entries,
            selected: 0,
            confirm_delete: None,
        }
    }

    fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    fn move_down(&mut self) {
        if self.selected + 1 < self.entries.len() {
            self.selected += 1;
        }
    }
}

pub fn handle_key(mut list: McpList, k: KeyEvent) -> (McpOutcome, Option<McpMenu>) {
    // Delete-confirmation sub-state takes priority.
    if let Some(idx) = list.confirm_delete {
        match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                let name = list
                    .entries
                    .get(idx)
                    .map(|e| e.name.clone())
                    .unwrap_or_default();
                return (McpOutcome::Save(delete_mcp_json(&name)), None);
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                list.confirm_delete = None;
            }
            _ => {}
        }
        return (McpOutcome::Idle, Some(McpMenu::List(list)));
    }

    // Empty list: only allow adding a new server.
    if list.entries.is_empty() {
        return match k.code {
            KeyCode::Char('n') => (McpOutcome::Idle, Some(McpMenu::Form(McpForm::new_blank()))),
            KeyCode::Esc => (McpOutcome::Cancel, None),
            _ => (McpOutcome::Idle, Some(McpMenu::List(list))),
        };
    }

    match k.code {
        KeyCode::Esc => (McpOutcome::Cancel, None),
        KeyCode::Up => {
            list.move_up();
            (McpOutcome::Idle, Some(McpMenu::List(list)))
        }
        KeyCode::Down => {
            list.move_down();
            (McpOutcome::Idle, Some(McpMenu::List(list)))
        }
        KeyCode::Enter => {
            let entry = &list.entries[list.selected];
            let json = toggle_mcp_json(&entry.name, !entry.enabled);
            (McpOutcome::Save(json), None)
        }
        KeyCode::Char('e') => {
            let entry = &list.entries[list.selected];
            (
                McpOutcome::Idle,
                Some(McpMenu::Form(McpForm::from_existing(entry))),
            )
        }
        KeyCode::Char('n') => (McpOutcome::Idle, Some(McpMenu::Form(McpForm::new_blank()))),
        KeyCode::Char('d') => {
            list.confirm_delete = Some(list.selected);
            (McpOutcome::Idle, Some(McpMenu::List(list)))
        }
        _ => (McpOutcome::Idle, Some(McpMenu::List(list))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use opencoder_core::config::McpServerConfig;
    use opencoder_core::Config;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn config_with_server(name: &str, enabled: bool) -> Config {
        let mut cfg = Config::default();
        cfg.mcp_servers.insert(
            name.to_string(),
            McpServerConfig {
                enabled,
                command: Some("npx".to_string()),
                ..Default::default()
            },
        );
        cfg
    }

    #[test]
    fn enter_toggles_enabled_on_selected() {
        let list = McpList::new(&config_with_server("srv", false));
        let (outcome, next) = handle_key(list, key(KeyCode::Enter));
        match outcome {
            McpOutcome::Save(json) => {
                assert_eq!(json["mcp_servers"]["srv"]["enabled"], true);
            }
            _ => panic!("expected Save"),
        }
        assert!(next.is_none());
    }

    #[test]
    fn enter_toggles_disabled_when_already_enabled() {
        let list = McpList::new(&config_with_server("srv", true));
        let (outcome, _) = handle_key(list, key(KeyCode::Enter));
        match outcome {
            McpOutcome::Save(json) => assert_eq!(json["mcp_servers"]["srv"]["enabled"], false),
            _ => panic!("expected Save"),
        }
    }

    #[test]
    fn new_on_empty_list_opens_form() {
        let list = McpList::new(&Config::default());
        let (outcome, next) = handle_key(list, key(KeyCode::Char('n')));
        assert!(matches!(outcome, McpOutcome::Idle));
        assert!(matches!(next, Some(McpMenu::Form(_))));
    }

    #[test]
    fn escape_cancels() {
        let list = McpList::new(&config_with_server("srv", false));
        let (outcome, next) = handle_key(list, key(KeyCode::Esc));
        assert!(matches!(outcome, McpOutcome::Cancel));
        assert!(next.is_none());
    }

    #[test]
    fn delete_then_y_saves_deletion() {
        let list = McpList::new(&config_with_server("srv", false));
        // First press 'd' to arm delete confirmation.
        let (outcome1, next1) = handle_key(list, key(KeyCode::Char('d')));
        assert!(matches!(outcome1, McpOutcome::Idle));
        let menu = next1.unwrap();
        let list = match menu {
            McpMenu::List(l) => l,
            _ => panic!("expected List"),
        };
        assert_eq!(list.confirm_delete, Some(0));
        // Then press 'y' to confirm.
        let (outcome2, next2) = handle_key(list, key(KeyCode::Char('y')));
        match outcome2 {
            McpOutcome::Save(json) => assert!(json["mcp_servers"]["srv"].is_null()),
            _ => panic!("expected Save"),
        }
        assert!(next2.is_none());
    }

    #[test]
    fn arrow_keys_navigate_selection() {
        let mut cfg = Config::default();
        cfg.mcp_servers
            .insert("a".to_string(), McpServerConfig::default());
        cfg.mcp_servers
            .insert("b".to_string(), McpServerConfig::default());
        let list = McpList::new(&cfg);
        assert_eq!(list.selected, 0);
        let (outcome, next) = handle_key(list, key(KeyCode::Down));
        assert!(matches!(outcome, McpOutcome::Idle));
        if let Some(McpMenu::List(l)) = next {
            assert_eq!(l.selected, 1);
        } else {
            panic!("expected List");
        }
    }

    #[test]
    fn edit_key_opens_form_with_existing() {
        let list = McpList::new(&config_with_server("srv", true));
        let (outcome, next) = handle_key(list, key(KeyCode::Char('e')));
        assert!(matches!(outcome, McpOutcome::Idle));
        if let Some(McpMenu::Form(f)) = next {
            assert_eq!(f.name, "srv");
            assert!(f.enabled);
        } else {
            panic!("expected Form");
        }
    }
}

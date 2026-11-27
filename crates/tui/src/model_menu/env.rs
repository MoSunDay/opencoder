//! Local named configuration Env manager and lifecycle operations.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};
use opencoder_core::{
    activate_config_env, config_env_status, create_config_env, deactivate_config_env,
    delete_config_env, list_config_envs, Config, ConfigEnvInfo,
};

use crate::command::EnvCommand;

use super::state::{ModelMenu, ModelOutcome};

type Transition = (ModelOutcome, Option<ModelMenu>);

pub struct EnvMenu {
    pub entries: Vec<ConfigEnvInfo>,
    pub selected: usize,
    pub confirm_delete: Option<usize>,
    pub create_input: Option<String>,
    pub notice: Option<String>,
    pub error: Option<String>,
    pub stale_marker: bool,
    workdir: PathBuf,
    pending: Option<EnvCommand>,
}

impl EnvMenu {
    pub fn new(workdir: &Path) -> Self {
        let mut menu = Self {
            entries: Vec::new(),
            selected: 0,
            confirm_delete: None,
            create_input: None,
            notice: None,
            error: None,
            stale_marker: false,
            workdir: workdir.to_path_buf(),
            pending: None,
        };
        menu.refresh();
        menu
    }

    pub fn with_command(workdir: &Path, command: EnvCommand) -> Self {
        let mut menu = Self::new(workdir);
        menu.pending = Some(command);
        menu
    }

    pub(crate) fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn paste_into(&mut self, text: &str) {
        if let Some(input) = &mut self.create_input {
            input.extend(
                text.chars()
                    .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_')),
            );
        }
    }

    fn refresh(&mut self) {
        let selected = self.selected_name();
        match list_config_envs() {
            Ok(entries) => {
                self.entries = entries;
                self.selected = selected
                    .and_then(|name| self.entries.iter().position(|env| env.name == name))
                    .or_else(|| self.entries.iter().position(|env| env.active))
                    .unwrap_or(0)
                    .min(self.entries.len().saturating_sub(1));
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
        self.stale_marker = config_env_status()
            .map(|status| status.stale_marker)
            .unwrap_or(false);
    }

    fn selected_name(&self) -> Option<String> {
        self.entries.get(self.selected).map(|env| env.name.clone())
    }

    fn idle(mut menu: Self) -> Transition {
        menu.refresh();
        (ModelOutcome::Idle, Some(ModelMenu::Env(menu)))
    }

    fn fail(mut self, error: impl std::fmt::Display) -> Transition {
        self.error = Some(format!("{error:#}"));
        self.refresh();
        (ModelOutcome::Idle, Some(ModelMenu::Env(self)))
    }

    fn reload(self, label: String) -> Transition {
        match Config::load(&self.workdir) {
            Ok(config) => (ModelOutcome::ReloadEnv(Box::new(config), label), None),
            Err(error) => self.fail(format_args!(
                "Env loaded but config reload failed: {error:#}"
            )),
        }
    }

    fn execute(mut self, command: EnvCommand) -> Transition {
        self.error = None;
        self.notice = None;
        match command {
            EnvCommand::Show => Self::idle(self),
            EnvCommand::Activate(name) => match activate_config_env(&name) {
                Ok(_) => self.reload(format!("activated Env `{name}`")),
                Err(error) => self.fail(error),
            },
            EnvCommand::Create(name) => match create_config_env(&name) {
                Ok(_) => {
                    self.notice = Some(format!("created Env `{name}`; press Enter to activate"));
                    Self::idle(self)
                }
                Err(error) => self.fail(error),
            },
            EnvCommand::Deactivate => match deactivate_config_env() {
                Ok(_) => self.reload("deactivated the local Env".into()),
                Err(error) => self.fail(error),
            },
            EnvCommand::Delete(name) => {
                let active = self
                    .entries
                    .iter()
                    .any(|env| env.active && env.name == name);
                match delete_config_env(&name) {
                    Ok(()) if active => self.reload(format!("deleted active Env `{name}`")),
                    Ok(()) => {
                        self.notice = Some(format!("deleted Env `{name}`"));
                        Self::idle(self)
                    }
                    Err(error) => self.fail(error),
                }
            }
            EnvCommand::Invalid(message) => self.fail(message),
        }
    }
}

fn handle_create(mut menu: EnvMenu, key: KeyEvent) -> Transition {
    match key.code {
        KeyCode::Esc => {
            menu.create_input = None;
            EnvMenu::idle(menu)
        }
        KeyCode::Enter => {
            let name = menu.create_input.take().unwrap_or_default();
            menu.execute(EnvCommand::Create(name))
        }
        KeyCode::Backspace => {
            if let Some(input) = &mut menu.create_input {
                input.pop();
            }
            (ModelOutcome::Idle, Some(ModelMenu::Env(menu)))
        }
        KeyCode::Char(character) => {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                if let Some(input) = &mut menu.create_input {
                    input.push(character);
                }
            }
            (ModelOutcome::Idle, Some(ModelMenu::Env(menu)))
        }
        _ => (ModelOutcome::Idle, Some(ModelMenu::Env(menu))),
    }
}

pub fn handle_key(mut menu: EnvMenu, key: KeyEvent) -> Transition {
    if let Some(command) = menu.pending.take() {
        return menu.execute(command);
    }
    if menu.create_input.is_some() {
        return handle_create(menu, key);
    }
    if let Some(index) = menu.confirm_delete {
        return match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                menu.confirm_delete = None;
                match menu.entries.get(index).map(|env| env.name.clone()) {
                    Some(name) => menu.execute(EnvCommand::Delete(name)),
                    None => EnvMenu::idle(menu),
                }
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                menu.confirm_delete = None;
                (ModelOutcome::Idle, Some(ModelMenu::Env(menu)))
            }
            _ => (ModelOutcome::Idle, Some(ModelMenu::Env(menu))),
        };
    }
    match key.code {
        KeyCode::Esc => (ModelOutcome::Cancel, None),
        KeyCode::Up => {
            menu.selected = menu.selected.saturating_sub(1);
            (ModelOutcome::Idle, Some(ModelMenu::Env(menu)))
        }
        KeyCode::Down => {
            if menu.selected + 1 < menu.entries.len() {
                menu.selected += 1;
            }
            (ModelOutcome::Idle, Some(ModelMenu::Env(menu)))
        }
        KeyCode::Enter => match menu.selected_name() {
            Some(name) => menu.execute(EnvCommand::Activate(name)),
            None => EnvMenu::idle(menu),
        },
        KeyCode::Char('n') | KeyCode::Char('N') => {
            menu.create_input = Some(String::new());
            menu.error = None;
            (ModelOutcome::Idle, Some(ModelMenu::Env(menu)))
        }
        KeyCode::Char('o') | KeyCode::Char('O') => menu.execute(EnvCommand::Deactivate),
        KeyCode::Char('d') | KeyCode::Char('D') => {
            if !menu.entries.is_empty() {
                menu.confirm_delete = Some(menu.selected);
            }
            (ModelOutcome::Idle, Some(ModelMenu::Env(menu)))
        }
        _ => (ModelOutcome::Idle, Some(ModelMenu::Env(menu))),
    }
}

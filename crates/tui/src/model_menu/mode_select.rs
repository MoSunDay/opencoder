//! Reusable Shared/Act/Plan selector for `/model` and `/config`.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};
use opencoder_core::{Config, ConfigMode, ConfigSaveRoute, ConfigSaveTarget};
use opencoder_store::SessionModelOverrides;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::config_form::ConfigForm;
use super::list::ProviderList;
use super::mode_config_form::ModeConfigForm;
use super::state::{ModelMenu, ModelOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeMenuKind {
    Model,
    Config,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveTargetInfo {
    pub path: String,
    pub active_env: Option<String>,
    pub route: ConfigSaveRoute,
}

impl SaveTargetInfo {
    pub fn resolve(workdir: &Path, requested: ConfigSaveRoute) -> Self {
        // The active Env is display state, not the save destination. Legacy
        // Shared config saves intentionally retain their project/global route.
        let active_env = opencoder_core::config_env_status()
            .ok()
            .and_then(|status| status.active);
        match Config::save_target_for(workdir, requested) {
            Ok(ConfigSaveTarget { path, route, .. }) => Self {
                path: path.display().to_string(),
                active_env,
                route,
            },
            Err(error) => Self {
                path: format!("unavailable: {error:#}"),
                active_env,
                route: requested,
            },
        }
    }

    pub fn env_label(&self) -> &str {
        self.active_env.as_deref().unwrap_or("none")
    }
}

#[derive(Clone)]
pub struct ModeSelection {
    pub config: Config,
    pub selected: ConfigMode,
    pub kind: ModeMenuKind,
    pub save_target: SaveTargetInfo,
    pub session_models: Option<SessionModelOverrides>,
    workdir: PathBuf,
}

fn save_route(kind: ModeMenuKind, mode: ConfigMode) -> ConfigSaveRoute {
    match (kind, mode) {
        (ModeMenuKind::Config, ConfigMode::Shared) => ConfigSaveRoute::Auto,
        (ModeMenuKind::Model, _) | (ModeMenuKind::Config, ConfigMode::Act | ConfigMode::Plan) => {
            ConfigSaveRoute::EnvAware
        }
    }
}

impl ModeSelection {
    pub fn new(
        config: Config,
        selected: ConfigMode,
        kind: ModeMenuKind,
        workdir: &Path,
        session_models: Option<SessionModelOverrides>,
    ) -> Self {
        let save_target = SaveTargetInfo::resolve(workdir, save_route(kind, selected));
        Self {
            save_target,
            config,
            selected,
            kind,
            session_models,
            workdir: workdir.to_path_buf(),
        }
    }

    fn refresh_save_target(&mut self) {
        self.save_target =
            SaveTargetInfo::resolve(&self.workdir, save_route(self.kind, self.selected));
    }

    pub fn effective(&self, mode: ConfigMode) -> Config {
        crate::model_scope::effective_for_mode(
            &self.config,
            &self.config.model,
            self.session_models.as_ref(),
            mode,
        )
    }

    fn enter(mut self) -> ModelMenu {
        self.refresh_save_target();
        match (self.kind, self.selected) {
            (ModeMenuKind::Model, mode) => ModelMenu::List(ProviderList::for_mode(self, mode)),
            (ModeMenuKind::Config, ConfigMode::Shared) => {
                let mut form = ConfigForm::new(&self.config);
                form.save_target = Some(self.save_target.clone());
                form.return_scope = Some(Box::new(self));
                ModelMenu::Config(form)
            }
            (ModeMenuKind::Config, mode) => ModelMenu::ModeConfig(ModeConfigForm::new(self, mode)),
        }
    }
}

pub fn handle_key(
    mut selection: ModeSelection,
    key: KeyEvent,
) -> (ModelOutcome, Option<ModelMenu>) {
    match key.code {
        KeyCode::Esc => (ModelOutcome::Cancel, None),
        KeyCode::Up => {
            selection.selected = match selection.selected {
                ConfigMode::Shared => ConfigMode::Plan,
                ConfigMode::Act => ConfigMode::Shared,
                ConfigMode::Plan => ConfigMode::Act,
            };
            selection.refresh_save_target();
            (
                ModelOutcome::Idle,
                Some(ModelMenu::ModeSelect(Box::new(selection))),
            )
        }
        KeyCode::Down => {
            selection.selected = match selection.selected {
                ConfigMode::Shared => ConfigMode::Act,
                ConfigMode::Act => ConfigMode::Plan,
                ConfigMode::Plan => ConfigMode::Shared,
            };
            selection.refresh_save_target();
            (
                ModelOutcome::Idle,
                Some(ModelMenu::ModeSelect(Box::new(selection))),
            )
        }
        KeyCode::Enter => (ModelOutcome::Idle, Some(selection.enter())),
        _ => (
            ModelOutcome::Idle,
            Some(ModelMenu::ModeSelect(Box::new(selection))),
        ),
    }
}

pub(crate) fn render(f: &mut Frame, area: Rect, composer_top: u16, selection: &ModeSelection) {
    let h = 12u16.min(composer_top.max(1));
    let w = 76u16.min(area.width.saturating_sub(4));
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        composer_top.saturating_sub(h),
        w,
        h,
    );
    f.render_widget(Clear, popup);
    let title = match selection.kind {
        ModeMenuKind::Model => " Model scope ",
        ModeMenuKind::Config => " Config scope ",
    };
    let mut lines = vec![Line::styled(
        "Choose Shared, Act, or Plan",
        Style::default().add_modifier(Modifier::BOLD),
    )];
    for mode in [ConfigMode::Shared, ConfigMode::Act, ConfigMode::Plan] {
        let marker = if selection.selected == mode {
            "❯"
        } else {
            " "
        };
        let style = if selection.selected == mode {
            Style::default().fg(crate::theme::warn_color())
        } else {
            Style::default().fg(crate::theme::text())
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{marker} {:<7}", crate::model_scope::mode_name(mode)),
                style,
            ),
            Span::styled(
                selection.effective(mode).model,
                Style::default().fg(crate::theme::subtle()),
            ),
        ]));
    }
    lines.extend([
        Line::raw(""),
        Line::raw(format!("Active Env: {}", selection.save_target.env_label())),
        Line::raw(format!("Save target: {}", selection.save_target.path)),
        Line::styled(
            "↑/↓ select · Enter continue · Esc cancel",
            Style::default().fg(crate::theme::accent()),
        ),
    ]);
    f.render_widget(
        Paragraph::new(lines).block(
            ratatui::widgets::Block::bordered()
                .title(title)
                .border_style(Style::default().fg(crate::theme::accent())),
        ),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_defaults_and_cycles() {
        let dir = tempfile::tempdir().unwrap();
        let selection = ModeSelection::new(
            Config::default(),
            ConfigMode::Act,
            ModeMenuKind::Model,
            dir.path(),
            Some(SessionModelOverrides::default()),
        );
        let mut slot = Some(ModelMenu::ModeSelect(Box::new(selection)));
        let outcome = handle_model_key_for_test(&mut slot, KeyCode::Down);
        assert!(matches!(outcome, ModelOutcome::Idle));
        let Some(ModelMenu::ModeSelect(next)) = slot else {
            panic!("selector remains open")
        };
        assert_eq!(next.selected, ConfigMode::Plan);
    }

    fn handle_model_key_for_test(slot: &mut Option<ModelMenu>, code: KeyCode) -> ModelOutcome {
        let key = KeyEvent::new(code, crossterm::event::KeyModifiers::NONE);
        match slot.take() {
            Some(ModelMenu::ModeSelect(selection)) => {
                let (outcome, next) = handle_key(*selection, key);
                *slot = next;
                outcome
            }
            _ => panic!("expected mode selector"),
        }
    }
}

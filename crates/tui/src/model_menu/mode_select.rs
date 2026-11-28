//! Act/Plan selector for `/model` and Shared/Act/Plan selector for `/config`.

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

impl ModeMenuKind {
    /// `/model` only edits Act/Plan models; Shared remains a config-file
    /// fallback and is still selectable for `/config` generation settings.
    pub fn modes(self) -> &'static [ConfigMode] {
        const MODEL_MODES: [ConfigMode; 2] = [ConfigMode::Act, ConfigMode::Plan];
        const CONFIG_MODES: [ConfigMode; 3] =
            [ConfigMode::Shared, ConfigMode::Act, ConfigMode::Plan];
        match self {
            Self::Model => &MODEL_MODES,
            Self::Config => &CONFIG_MODES,
        }
    }
}

fn normalize_mode(kind: ModeMenuKind, mode: ConfigMode) -> ConfigMode {
    let modes = kind.modes();
    if modes.contains(&mode) {
        mode
    } else {
        modes[0]
    }
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
        let selected = normalize_mode(kind, selected);
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
        self.selected = normalize_mode(self.kind, self.selected);
        self.save_target =
            SaveTargetInfo::resolve(&self.workdir, save_route(self.kind, self.selected));
    }

    /// Display value for a scope row. Act/Plan inherit the effective base
    /// model until the user saves a mode-specific pick, so both rows always
    /// carry a concrete value in `/model`.
    pub fn mode_model(&self, mode: ConfigMode) -> String {
        let model = self.effective(mode).model;
        if !model.trim().is_empty() {
            return model;
        }
        let base = self.config.model.trim();
        if base.is_empty() {
            "(unset)".to_string()
        } else {
            base.to_string()
        }
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
            cycle_selected(&mut selection, -1);
            (
                ModelOutcome::Idle,
                Some(ModelMenu::ModeSelect(Box::new(selection))),
            )
        }
        KeyCode::Down => {
            cycle_selected(&mut selection, 1);
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

fn cycle_selected(selection: &mut ModeSelection, delta: isize) {
    let modes = selection.kind.modes();
    if modes.is_empty() {
        return;
    }
    let index = modes
        .iter()
        .position(|mode| *mode == selection.selected)
        .unwrap_or(0);
    let next = (index as isize + delta).rem_euclid(modes.len() as isize) as usize;
    selection.selected = modes[next];
    selection.refresh_save_target();
}

pub(crate) fn render(f: &mut Frame, area: Rect, composer_top: u16, selection: &ModeSelection) {
    let modes = selection.kind.modes();
    let h = (9 + modes.len() as u16).min(composer_top.max(1));
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
    let prompt = match selection.kind {
        ModeMenuKind::Model => "Choose Act or Plan",
        ModeMenuKind::Config => "Choose Shared, Act, or Plan",
    };
    let mut lines = vec![Line::styled(
        prompt,
        Style::default().add_modifier(Modifier::BOLD),
    )];
    for mode in modes {
        let marker = if selection.selected == *mode {
            "❯"
        } else {
            " "
        };
        let style = if selection.selected == *mode {
            Style::default().fg(crate::theme::warn_color())
        } else {
            Style::default().fg(crate::theme::text())
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{marker} {:<7}", crate::model_scope::mode_name(*mode)),
                style,
            ),
            Span::styled(
                selection.mode_model(*mode),
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
        let mut slot = Some(ModelMenu::ModeSelect(next));
        let outcome = handle_model_key_for_test(&mut slot, KeyCode::Down);
        assert!(matches!(outcome, ModelOutcome::Idle));
        let Some(ModelMenu::ModeSelect(next)) = slot else {
            panic!("selector remains open")
        };
        assert_eq!(next.selected, ConfigMode::Act, "wraps without Shared");
    }

    #[test]
    fn model_selector_lists_only_act_and_plan() {
        assert_eq!(
            ModeMenuKind::Model.modes(),
            &[ConfigMode::Act, ConfigMode::Plan]
        );
        assert_eq!(
            ModeMenuKind::Config.modes(),
            &[ConfigMode::Shared, ConfigMode::Act, ConfigMode::Plan]
        );
    }

    #[test]
    fn model_selector_normalizes_shared_to_act() {
        let dir = tempfile::tempdir().unwrap();
        let model = ModeSelection::new(
            Config::default(),
            ConfigMode::Shared,
            ModeMenuKind::Model,
            dir.path(),
            Some(SessionModelOverrides::default()),
        );
        assert_eq!(model.selected, ConfigMode::Act);

        let config = ModeSelection::new(
            Config::default(),
            ConfigMode::Shared,
            ModeMenuKind::Config,
            dir.path(),
            Some(SessionModelOverrides::default()),
        );
        assert_eq!(config.selected, ConfigMode::Shared);
    }

    #[test]
    fn model_rows_have_inherited_values_for_both_modes() {
        let dir = tempfile::tempdir().unwrap();
        let selection = ModeSelection::new(
            Config {
                model: "openai/base".into(),
                ..Default::default()
            },
            ConfigMode::Act,
            ModeMenuKind::Model,
            dir.path(),
            Some(SessionModelOverrides::default()),
        );
        assert_eq!(selection.mode_model(ConfigMode::Act), "openai/base");
        assert_eq!(selection.mode_model(ConfigMode::Plan), "openai/base");
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

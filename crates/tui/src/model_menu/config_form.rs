//! Slim `/config` form: generation parameters only (no model/base_url/api_key
//! — those moved to `/model`).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::Config;

use super::patch::ConfigPatch;
use super::state::{ModelMenu, ModelOutcome};

/// Reasoning-effort selector state. `Off` serializes to `null` (omit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reasoning {
    Off,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl Reasoning {
    pub fn label(self) -> &'static str {
        match self {
            Reasoning::Off => "off",
            Reasoning::Low => "low",
            Reasoning::Medium => "medium",
            Reasoning::High => "high",
            Reasoning::XHigh => "xhigh",
            Reasoning::Max => "max",
        }
    }
    pub fn next(self) -> Self {
        match self {
            Reasoning::Off => Reasoning::Low,
            Reasoning::Low => Reasoning::Medium,
            Reasoning::Medium => Reasoning::High,
            Reasoning::High => Reasoning::XHigh,
            Reasoning::XHigh => Reasoning::Max,
            Reasoning::Max => Reasoning::Off,
        }
    }
    pub fn prev(self) -> Self {
        match self {
            Reasoning::Off => Reasoning::Max,
            Reasoning::Low => Reasoning::Off,
            Reasoning::Medium => Reasoning::Low,
            Reasoning::High => Reasoning::Medium,
            Reasoning::XHigh => Reasoning::High,
            Reasoning::Max => Reasoning::XHigh,
        }
    }
    pub fn from_config(v: Option<&str>) -> Self {
        match v.map(|s| s.trim().to_lowercase()).as_deref() {
            Some("low") => Reasoning::Low,
            Some("medium") => Reasoning::Medium,
            Some("high") => Reasoning::High,
            Some("xhigh") => Reasoning::XHigh,
            Some("max") => Reasoning::Max,
            _ => Reasoning::Off,
        }
    }
    pub fn to_option(self) -> Option<String> {
        match self {
            Reasoning::Off => None,
            Reasoning::Low => Some("low".into()),
            Reasoning::Medium => Some("medium".into()),
            Reasoning::High => Some("high".into()),
            Reasoning::XHigh => Some("xhigh".into()),
            Reasoning::Max => Some("max".into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigField {
    Reasoning,
    InterleavedThinking,
    MaxTokens,
    ContextSize,
    Threshold,
    Fps,
    Browser,
    ComputerUse,
    ToolsSubagent,
    ApMaxIter,
    Theme,
    EnableTmuxSession,
    Save,
    Cancel,
}

impl ConfigField {
    const ORDER: [ConfigField; 14] = [
        ConfigField::Reasoning,
        ConfigField::InterleavedThinking,
        ConfigField::MaxTokens,
        ConfigField::ContextSize,
        ConfigField::Threshold,
        ConfigField::Fps,
        ConfigField::Browser,
        ConfigField::ComputerUse,
        ConfigField::ToolsSubagent,
        ConfigField::ApMaxIter,
        ConfigField::Theme,
        ConfigField::EnableTmuxSession,
        ConfigField::Save,
        ConfigField::Cancel,
    ];
    pub fn next(self) -> Self {
        let i = Self::ORDER.iter().position(|&f| f == self).unwrap_or(0);
        Self::ORDER[(i + 1) % Self::ORDER.len()]
    }
    pub fn prev(self) -> Self {
        let i = Self::ORDER.iter().position(|&f| f == self).unwrap_or(0);
        Self::ORDER[(i + Self::ORDER.len() - 1) % Self::ORDER.len()]
    }
}

/// Parse a numeric field's String buffer. Empty/unparseable → `None`.
fn parse_field(s: &str) -> Option<u64> {
    s.trim().parse::<u64>().ok()
}

pub struct ConfigForm {
    pub reasoning: Reasoning,
    pub interleaved_thinking: bool,
    pub max_tokens_input: String,
    /// Char-index edit cursor within `max_tokens_input`.
    pub max_tokens_cursor: usize,
    pub threshold_input: String,
    /// Char-index edit cursor within `threshold_input`.
    pub threshold_cursor: usize,
    pub context_size_input: String,
    /// Char-index edit cursor within `context_size_input`.
    pub context_size_cursor: usize,
    pub fps_input: String,
    /// Char-index edit cursor within `fps_input`.
    pub fps_cursor: usize,
    pub capabilities_browser: bool,
    pub capabilities_computer_use: bool,
    pub capabilities_tools_subagent: bool,
    pub ap_max_iter_input: String,
    /// Char-index edit cursor within `ap_max_iter_input`.
    pub ap_max_iter_cursor: usize,
    pub theme: crate::theme::ThemeKind,
    pub enable_tmux_session: bool,
    pub focus: ConfigField,
    pub error: Option<String>,
}

impl ConfigForm {
    pub fn new(config: &Config) -> Self {
        // Cursors start at the end of each buffer so plain typing appends,
        // preserving the pre-cursor editing model.
        let max_tokens_input = config.max_tokens.map(|v| v.to_string()).unwrap_or_default();
        let threshold_input = config.compaction.context_threshold.to_string();
        let context_size_input = config.context_limit().to_string();
        let fps_input = config.tui_fps().to_string();
        let ap_max_iter_input = config.autopilot.max_iterations.to_string();
        ConfigForm {
            reasoning: Reasoning::from_config(config.reasoning_effort.as_deref()),
            interleaved_thinking: config.interleaved_thinking.unwrap_or(true),
            max_tokens_input: max_tokens_input.clone(),
            max_tokens_cursor: max_tokens_input.chars().count(),
            threshold_input: threshold_input.clone(),
            threshold_cursor: threshold_input.chars().count(),
            context_size_input: context_size_input.clone(),
            context_size_cursor: context_size_input.chars().count(),
            fps_input: fps_input.clone(),
            fps_cursor: fps_input.chars().count(),
            capabilities_browser: config.capabilities.browser,
            capabilities_computer_use: config.capabilities.computer_use,
            capabilities_tools_subagent: config.capabilities.tools_subagent,
            ap_max_iter_input: ap_max_iter_input.clone(),
            ap_max_iter_cursor: ap_max_iter_input.chars().count(),
            theme: crate::theme::ThemeKind::from_label(&config.theme),
            enable_tmux_session: config.enable_tmux_session.unwrap_or(false),
            focus: ConfigField::Reasoning,
            error: None,
        }
    }

    /// Apply `op` to the focused numeric field's (text, cursor). Toggle and
    /// button fields are no-ops. Pure editing: only the text buffer and its
    /// cursor move — no value interpretation happens here.
    fn edit_numeric<F>(&mut self, op: F)
    where
        F: FnOnce(&mut String, &mut usize),
    {
        match self.focus {
            ConfigField::MaxTokens => op(&mut self.max_tokens_input, &mut self.max_tokens_cursor),
            ConfigField::ContextSize => {
                op(&mut self.context_size_input, &mut self.context_size_cursor)
            }
            ConfigField::Threshold => op(&mut self.threshold_input, &mut self.threshold_cursor),
            ConfigField::Fps => op(&mut self.fps_input, &mut self.fps_cursor),
            ConfigField::ApMaxIter => op(&mut self.ap_max_iter_input, &mut self.ap_max_iter_cursor),
            _ => {}
        }
    }

    pub fn build_patch(&self) -> ConfigPatch {
        let max_tokens = if self.max_tokens_input.trim().is_empty() {
            None
        } else {
            self.max_tokens_input.trim().parse::<u64>().ok()
        };
        // Empty/unparseable → fall back to safe defaults (validate() blocks
        // empties on the save path; this is the safety net for direct callers).
        let threshold = parse_field(&self.threshold_input).unwrap_or(1000).max(1000);
        let context_size = parse_field(&self.context_size_input)
            .unwrap_or(128_000)
            .max(1);
        let fps = self
            .fps_input
            .trim()
            .parse::<u32>()
            .unwrap_or(10)
            .clamp(1, 30);
        let ap_max_iter = self
            .ap_max_iter_input
            .trim()
            .parse::<u32>()
            .unwrap_or(10)
            .max(1);
        ConfigPatch {
            reasoning_effort: self.reasoning.to_option(),
            interleaved_thinking: Some(self.interleaved_thinking),
            max_tokens,
            context_threshold: threshold,
            context_limit: context_size,
            fps,
            capabilities_browser: self.capabilities_browser,
            capabilities_computer_use: self.capabilities_computer_use,
            capabilities_tools_subagent: self.capabilities_tools_subagent,
            ap_max_iter,
            theme: self.theme.label().to_string(),
            enable_tmux_session: Some(self.enable_tmux_session),
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.context_size_input.trim().is_empty() {
            return Err("context_size cannot be empty".into());
        }
        if self.threshold_input.trim().is_empty() {
            return Err("context_threshold cannot be empty".into());
        }
        if self.fps_input.trim().is_empty() {
            return Err("fps cannot be empty".into());
        }
        if self.ap_max_iter_input.trim().is_empty() {
            return Err("ap_max_iter cannot be empty".into());
        }
        let threshold = parse_field(&self.threshold_input)
            .ok_or_else(|| "context_threshold is not a number".to_string())?;
        let context_size = parse_field(&self.context_size_input)
            .ok_or_else(|| "context_size is not a number".to_string())?;
        if threshold < 1000 {
            return Err("context_threshold must be >= 1000".into());
        }
        if threshold > context_size {
            return Err("context_threshold must not exceed context size".into());
        }
        Ok(())
    }

    /// Paste digits into the focused numeric field at the cursor (mirrors the
    /// `Char` digit filter; non-digits are dropped).
    pub fn paste_into(&mut self, text: &str) {
        let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return;
        }
        self.edit_numeric(|input, cur| {
            let idx = (*cur).min(input.chars().count());
            let (s, i) = crate::composer::insert_str(input, idx, &digits);
            *input = s;
            *cur = i;
        });
    }
}

/// Handle a key in `/config` mode. Takes ownership, returns outcome + next menu.
pub fn handle_key(mut form: ConfigForm, k: KeyEvent) -> (ModelOutcome, Option<ModelMenu>) {
    form.error = None;
    // Ctrl+L / Ctrl+U: clear the focused numeric field (max_tokens /
    // context_size / threshold / fps / ap_max_iter). No-op on toggle and
    // button fields. Both the 'l'/'u' char form and the raw control-char
    // forms (\u{c} FF, \u{15} NAK, per kitty keyboard protocol) match.
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        match k.code {
            KeyCode::Char('l')
            | KeyCode::Char('\u{c}')
            | KeyCode::Char('u')
            | KeyCode::Char('\u{15}') => form.edit_numeric(|text, cur| {
                text.clear();
                *cur = 0;
            }),
            _ => {}
        }
        return (ModelOutcome::Idle, Some(ModelMenu::Config(form)));
    }
    match k.code {
        KeyCode::Esc => return (ModelOutcome::Cancel, None),
        KeyCode::Tab => form.focus = form.focus.next(),
        KeyCode::BackTab => form.focus = form.focus.prev(),
        KeyCode::Up => form.focus = form.focus.prev(),
        KeyCode::Down => form.focus = form.focus.next(),
        KeyCode::Left => match form.focus {
            ConfigField::Reasoning => form.reasoning = form.reasoning.prev(),
            ConfigField::InterleavedThinking => {
                form.interleaved_thinking = !form.interleaved_thinking
            }
            ConfigField::Browser => form.capabilities_browser = !form.capabilities_browser,
            ConfigField::ComputerUse => {
                form.capabilities_computer_use = !form.capabilities_computer_use
            }
            ConfigField::ToolsSubagent => {
                form.capabilities_tools_subagent = !form.capabilities_tools_subagent
            }
            ConfigField::Theme => form.theme = form.theme.next(),
            ConfigField::EnableTmuxSession => form.enable_tmux_session = !form.enable_tmux_session,
            ConfigField::MaxTokens
            | ConfigField::ContextSize
            | ConfigField::Threshold
            | ConfigField::Fps
            | ConfigField::ApMaxIter => form.edit_numeric(|_, cur| *cur = cur.saturating_sub(1)),
            _ => {}
        },
        KeyCode::Right => match form.focus {
            ConfigField::Reasoning => form.reasoning = form.reasoning.next(),
            ConfigField::InterleavedThinking => {
                form.interleaved_thinking = !form.interleaved_thinking
            }
            ConfigField::Browser => form.capabilities_browser = !form.capabilities_browser,
            ConfigField::ComputerUse => {
                form.capabilities_computer_use = !form.capabilities_computer_use
            }
            ConfigField::ToolsSubagent => {
                form.capabilities_tools_subagent = !form.capabilities_tools_subagent
            }
            ConfigField::Theme => form.theme = form.theme.next(),
            ConfigField::EnableTmuxSession => form.enable_tmux_session = !form.enable_tmux_session,
            ConfigField::MaxTokens
            | ConfigField::ContextSize
            | ConfigField::Threshold
            | ConfigField::Fps
            | ConfigField::ApMaxIter => {
                form.edit_numeric(|text, cur| *cur = (*cur + 1).min(text.chars().count()));
            }
            _ => {}
        },
        KeyCode::Enter => match form.focus {
            ConfigField::Save => {
                if let Err(e) = form.validate() {
                    form.error = Some(e);
                    return (ModelOutcome::Idle, Some(ModelMenu::Config(form)));
                }
                let json = form.build_patch().to_json();
                return (ModelOutcome::Save(json), None);
            }
            ConfigField::Cancel => return (ModelOutcome::Cancel, None),
            _ => form.focus = form.focus.next(),
        },
        KeyCode::Backspace => form.edit_numeric(|text, cur| {
            let idx = (*cur).min(text.chars().count());
            if let Some((s, i)) = crate::composer::backspace(text, idx) {
                *text = s;
                *cur = i;
            }
        }),
        KeyCode::Char(c) => match form.focus {
            ConfigField::Reasoning if c == ' ' => form.reasoning = form.reasoning.next(),
            ConfigField::InterleavedThinking if c == ' ' => {
                form.interleaved_thinking = !form.interleaved_thinking
            }
            ConfigField::Browser if c == ' ' => {
                form.capabilities_browser = !form.capabilities_browser
            }
            ConfigField::ComputerUse if c == ' ' => {
                form.capabilities_computer_use = !form.capabilities_computer_use
            }
            ConfigField::ToolsSubagent if c == ' ' => {
                form.capabilities_tools_subagent = !form.capabilities_tools_subagent
            }
            ConfigField::Theme if c == ' ' => form.theme = form.theme.next(),
            ConfigField::EnableTmuxSession if c == ' ' => {
                form.enable_tmux_session = !form.enable_tmux_session
            }
            ConfigField::MaxTokens
            | ConfigField::ContextSize
            | ConfigField::Threshold
            | ConfigField::Fps
            | ConfigField::ApMaxIter
                if c.is_ascii_digit() =>
            {
                form.edit_numeric(|text, cur| {
                    let idx = (*cur).min(text.chars().count());
                    let (s, i) = crate::composer::insert_char(text, idx, c);
                    *text = s;
                    *cur = i;
                });
            }
            _ => {}
        },
        _ => {}
    }
    (ModelOutcome::Idle, Some(ModelMenu::Config(form)))
}

//! Slim `/config` form: generation parameters only (no model/base_url/api_key
//! — those moved to `/model`).

use crossterm::event::{KeyCode, KeyEvent};
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
    ApEnabled,
    ApMaxIter,
    Theme,
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
        ConfigField::ApEnabled,
        ConfigField::ApMaxIter,
        ConfigField::Theme,
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
    pub threshold_input: String,
    pub context_size_input: String,
    pub fps_input: String,
    pub capabilities_browser: bool,
    pub capabilities_computer_use: bool,
    pub capabilities_tools_subagent: bool,
    pub ap_enabled: bool,
    pub ap_max_iter_input: String,
    pub theme: crate::theme::ThemeKind,
    pub focus: ConfigField,
    pub error: Option<String>,
}

impl ConfigForm {
    pub fn new(config: &Config) -> Self {
        ConfigForm {
            reasoning: Reasoning::from_config(config.reasoning_effort.as_deref()),
            interleaved_thinking: config.interleaved_thinking.unwrap_or(true),
            max_tokens_input: config.max_tokens.map(|v| v.to_string()).unwrap_or_default(),
            threshold_input: config.compaction.context_threshold.to_string(),
            context_size_input: config.context_limit().to_string(),
            fps_input: config.tui_fps().to_string(),
            capabilities_browser: config.capabilities.browser,
            capabilities_computer_use: config.capabilities.computer_use,
            capabilities_tools_subagent: config.capabilities.tools_subagent,
            ap_enabled: config.autopilot.enabled,
            ap_max_iter_input: config.autopilot.max_iterations.to_string(),
            theme: crate::theme::ThemeKind::from_label(&config.theme),
            focus: ConfigField::Reasoning,
            error: None,
        }
    }

    fn adjust_threshold(&mut self, delta: i64) {
        let cur = self.threshold_input.parse::<i64>().unwrap_or(0);
        self.threshold_input = (cur + delta).max(0).to_string();
    }

    fn adjust_context_size(&mut self, delta: i64) {
        let cur = self.context_size_input.parse::<i64>().unwrap_or(0);
        self.context_size_input = (cur + delta).max(0).to_string();
    }

    fn adjust_fps(&mut self, delta: i32) {
        let cur = self.fps_input.parse::<i32>().unwrap_or(0);
        self.fps_input = (cur + delta).max(0).to_string();
    }

    fn adjust_ap_max_iter(&mut self, delta: i32) {
        let cur = self.ap_max_iter_input.parse::<i32>().unwrap_or(0);
        self.ap_max_iter_input = (cur + delta).max(0).to_string();
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
            ap_enabled: self.ap_enabled,
            ap_max_iter,
            theme: self.theme.label().to_string(),
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

    /// Paste text into the focused numeric field (mirrors the `Char` digit filter).
    pub fn paste_into(&mut self, text: &str) {
        for c in text.chars() {
            if !c.is_ascii_digit() {
                continue;
            }
            match self.focus {
                ConfigField::MaxTokens => self.max_tokens_input.push(c),
                ConfigField::ContextSize => self.context_size_input.push(c),
                ConfigField::Threshold => self.threshold_input.push(c),
                ConfigField::Fps => self.fps_input.push(c),
                ConfigField::ApMaxIter => self.ap_max_iter_input.push(c),
                _ => {}
            }
        }
    }
}

/// Handle a key in `/config` mode. Takes ownership, returns outcome + next menu.
pub fn handle_key(mut form: ConfigForm, k: KeyEvent) -> (ModelOutcome, Option<ModelMenu>) {
    form.error = None;
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
            ConfigField::ContextSize => form.adjust_context_size(-1000),
            ConfigField::Threshold => form.adjust_threshold(-1000),
            ConfigField::Fps => form.adjust_fps(-1),
            ConfigField::Browser => form.capabilities_browser = !form.capabilities_browser,
            ConfigField::ComputerUse => {
                form.capabilities_computer_use = !form.capabilities_computer_use
            }
            ConfigField::ToolsSubagent => {
                form.capabilities_tools_subagent = !form.capabilities_tools_subagent
            }
            ConfigField::ApEnabled => form.ap_enabled = !form.ap_enabled,
            ConfigField::ApMaxIter => form.adjust_ap_max_iter(-1),
            ConfigField::Theme => form.theme = form.theme.next(),
            _ => {}
        },
        KeyCode::Right => match form.focus {
            ConfigField::Reasoning => form.reasoning = form.reasoning.next(),
            ConfigField::InterleavedThinking => {
                form.interleaved_thinking = !form.interleaved_thinking
            }
            ConfigField::ContextSize => form.adjust_context_size(1000),
            ConfigField::Threshold => form.adjust_threshold(1000),
            ConfigField::Fps => form.adjust_fps(1),
            ConfigField::Browser => form.capabilities_browser = !form.capabilities_browser,
            ConfigField::ComputerUse => {
                form.capabilities_computer_use = !form.capabilities_computer_use
            }
            ConfigField::ToolsSubagent => {
                form.capabilities_tools_subagent = !form.capabilities_tools_subagent
            }
            ConfigField::ApEnabled => form.ap_enabled = !form.ap_enabled,
            ConfigField::ApMaxIter => form.adjust_ap_max_iter(1),
            ConfigField::Theme => form.theme = form.theme.next(),
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
        KeyCode::Backspace => match form.focus {
            ConfigField::MaxTokens => {
                form.max_tokens_input.pop();
            }
            ConfigField::ContextSize => {
                form.context_size_input.pop();
            }
            ConfigField::Threshold => {
                form.threshold_input.pop();
            }
            ConfigField::Fps => {
                form.fps_input.pop();
            }
            ConfigField::ApMaxIter => {
                form.ap_max_iter_input.pop();
            }
            _ => {}
        },
        KeyCode::Char(c) => match form.focus {
            ConfigField::MaxTokens if c.is_ascii_digit() => {
                form.max_tokens_input.push(c);
            }
            ConfigField::ContextSize if c.is_ascii_digit() => {
                form.context_size_input.push(c);
            }
            ConfigField::Threshold if c.is_ascii_digit() => {
                form.threshold_input.push(c);
            }
            ConfigField::Fps if c.is_ascii_digit() => {
                form.fps_input.push(c);
            }
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
            ConfigField::ApEnabled if c == ' ' => form.ap_enabled = !form.ap_enabled,
            ConfigField::Theme if c == ' ' => form.theme = form.theme.next(),
            ConfigField::ApMaxIter if c.is_ascii_digit() => {
                form.ap_max_iter_input.push(c);
            }
            _ => {}
        },
        _ => {}
    }
    (ModelOutcome::Idle, Some(ModelMenu::Config(form)))
}

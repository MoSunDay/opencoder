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
}

impl Reasoning {
    pub fn label(self) -> &'static str {
        match self {
            Reasoning::Off => "off",
            Reasoning::Low => "low",
            Reasoning::Medium => "medium",
            Reasoning::High => "high",
        }
    }
    pub fn next(self) -> Self {
        match self {
            Reasoning::Off => Reasoning::Low,
            Reasoning::Low => Reasoning::Medium,
            Reasoning::Medium => Reasoning::High,
            Reasoning::High => Reasoning::Off,
        }
    }
    pub fn prev(self) -> Self {
        match self {
            Reasoning::Off => Reasoning::High,
            Reasoning::Low => Reasoning::Off,
            Reasoning::Medium => Reasoning::Low,
            Reasoning::High => Reasoning::Medium,
        }
    }
    pub fn from_config(v: Option<&str>) -> Self {
        match v.map(|s| s.trim().to_lowercase()).as_deref() {
            Some("low") => Reasoning::Low,
            Some("medium") => Reasoning::Medium,
            Some("high") => Reasoning::High,
            _ => Reasoning::Off,
        }
    }
    pub fn to_option(self) -> Option<String> {
        match self {
            Reasoning::Off => None,
            Reasoning::Low => Some("low".into()),
            Reasoning::Medium => Some("medium".into()),
            Reasoning::High => Some("high".into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigField {
    Reasoning,
    InterleavedThinking,
    MaxTokens,
    Threshold,
    Fps,
    Browser,
    ComputerUse,
    ToolsSubagent,
    Save,
    Cancel,
}

impl ConfigField {
    const ORDER: [ConfigField; 10] = [
        ConfigField::Reasoning,
        ConfigField::InterleavedThinking,
        ConfigField::MaxTokens,
        ConfigField::Threshold,
        ConfigField::Fps,
        ConfigField::Browser,
        ConfigField::ComputerUse,
        ConfigField::ToolsSubagent,
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

pub struct ConfigForm {
    pub reasoning: Reasoning,
    pub interleaved_thinking: bool,
    pub max_tokens_input: String,
    pub threshold: u64,
    pub fps: u32,
    pub capabilities_browser: bool,
    pub capabilities_computer_use: bool,
    pub capabilities_tools_subagent: bool,
    pub focus: ConfigField,
    pub error: Option<String>,
}

impl ConfigForm {
    pub fn new(config: &Config) -> Self {
        ConfigForm {
            reasoning: Reasoning::from_config(config.reasoning_effort.as_deref()),
            interleaved_thinking: config.interleaved_thinking.unwrap_or(true),
            max_tokens_input: config.max_tokens.map(|v| v.to_string()).unwrap_or_default(),
            threshold: config.compaction.context_threshold,
            fps: config.tui_fps(),
            capabilities_browser: config.capabilities.browser,
            capabilities_computer_use: config.capabilities.computer_use,
            capabilities_tools_subagent: config.capabilities.tools_subagent,
            focus: ConfigField::Reasoning,
            error: None,
        }
    }

    fn adjust_threshold(&mut self, delta: i64) {
        let next = self.threshold as i64 + delta;
        self.threshold = next.max(1000) as u64;
    }

    fn adjust_fps(&mut self, delta: i32) {
        self.fps = (self.fps as i32 + delta).clamp(1, 30) as u32;
    }

    pub fn build_patch(&self) -> ConfigPatch {
        let max_tokens = if self.max_tokens_input.trim().is_empty() {
            None
        } else {
            self.max_tokens_input.trim().parse::<u64>().ok()
        };
        ConfigPatch {
            reasoning_effort: self.reasoning.to_option(),
            interleaved_thinking: Some(self.interleaved_thinking),
            max_tokens,
            context_threshold: self.threshold,
            fps: self.fps,
            capabilities_browser: self.capabilities_browser,
            capabilities_computer_use: self.capabilities_computer_use,
            capabilities_tools_subagent: self.capabilities_tools_subagent,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.threshold < 1000 {
            return Err("context_threshold must be >= 1000".into());
        }
        Ok(())
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
            ConfigField::Threshold => form.adjust_threshold(-1000),
            ConfigField::Fps => form.adjust_fps(-1),
            ConfigField::Browser => form.capabilities_browser = !form.capabilities_browser,
            ConfigField::ComputerUse => {
                form.capabilities_computer_use = !form.capabilities_computer_use
            }
            ConfigField::ToolsSubagent => {
                form.capabilities_tools_subagent = !form.capabilities_tools_subagent
            }
            _ => {}
        },
        KeyCode::Right => match form.focus {
            ConfigField::Reasoning => form.reasoning = form.reasoning.next(),
            ConfigField::InterleavedThinking => {
                form.interleaved_thinking = !form.interleaved_thinking
            }
            ConfigField::Threshold => form.adjust_threshold(1000),
            ConfigField::Fps => form.adjust_fps(1),
            ConfigField::Browser => form.capabilities_browser = !form.capabilities_browser,
            ConfigField::ComputerUse => {
                form.capabilities_computer_use = !form.capabilities_computer_use
            }
            ConfigField::ToolsSubagent => {
                form.capabilities_tools_subagent = !form.capabilities_tools_subagent
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
        KeyCode::Backspace => {
            if matches!(form.focus, ConfigField::MaxTokens) {
                form.max_tokens_input.pop();
            }
        }
        KeyCode::Char(c) => match form.focus {
            ConfigField::MaxTokens if c.is_ascii_digit() => {
                form.max_tokens_input.push(c);
            }
            ConfigField::Threshold if c.is_ascii_digit() => {
                let s = format!("{}{}", form.threshold, c);
                if let Ok(n) = s.parse::<u64>() {
                    form.threshold = n.max(1000);
                }
            }
            ConfigField::Fps if c.is_ascii_digit() => {
                let s = format!("{}{}", form.fps, c);
                if let Ok(n) = s.parse::<u32>() {
                    form.fps = n.clamp(1, 30);
                }
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
            _ => {}
        },
        _ => {}
    }
    (ModelOutcome::Idle, Some(ModelMenu::Config(form)))
}

//! State + key handling for the `/config` modal. See [`crate::model_menu`] docs.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::looks_like_env_var;
use opencoder_core::Config;

/// Editable subset of config produced by the `/config` menu. `api_key: None`
/// means "leave the existing value untouched"; `Some(v)` replaces it.
#[derive(Debug, Clone)]
pub struct ModelPatch {
    pub model: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub reasoning_effort: Option<String>,
    pub interleaved_thinking: Option<bool>,
    pub context_threshold: u64,
    pub fps: u32,
    pub capabilities_browser: bool,
    pub capabilities_computer_use: bool,
}

impl ModelPatch {
    /// Build the JSON merge-patch consumed by `Config::save`.
    ///
    /// `api_key: None` (untouched) is **omitted** from the patch so the
    /// existing key is preserved; `Some("")` (explicit clear) becomes `null`
    /// (merge removes the key); `Some(v)` writes the value, wrapping
    /// env-var-shaped names as `{NAME}`.
    pub fn to_json(&self) -> serde_json::Value {
        let mut provider = serde_json::json!({ "base_url": self.base_url });
        if let Some(v) = &self.api_key {
            let v = v.trim();
            let resolved = if v.is_empty() {
                serde_json::Value::Null
            } else if looks_like_env_var(v) {
                serde_json::Value::String(format!("{{{v}}}"))
            } else {
                serde_json::Value::String(v.to_string())
            };
            provider["api_key"] = resolved;
        }
        serde_json::json!({
            "model": self.model,
            "provider": provider,
            "reasoning_effort": self.reasoning_effort,
            "interleaved_thinking": self.interleaved_thinking,
            "compaction": {
                "context_threshold": self.context_threshold,
            },
            "fps": self.fps,
            "capabilities": {
                "browser": self.capabilities_browser,
                "computer_use": self.capabilities_computer_use,
            },
        })
    }
}

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
            other => Some(other.label().to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Model,
    BaseUrl,
    ApiKey,
    Reasoning,
    InterleavedThinking,
    Threshold,
    Fps,
    Browser,
    ComputerUse,
    Save,
    Cancel,
}

impl Field {
    const ORDER: [Field; 11] = [
        Field::Model,
        Field::BaseUrl,
        Field::ApiKey,
        Field::Reasoning,
        Field::InterleavedThinking,
        Field::Threshold,
        Field::Fps,
        Field::Browser,
        Field::ComputerUse,
        Field::Save,
        Field::Cancel,
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

/// Outcome of a keystroke while the `/config` modal is open.
pub enum ModelOutcome {
    Idle,
    Save(ModelPatch),
    Cancel,
    Quit,
}

pub struct ModelMenu {
    pub model: String,
    pub base_url: String,
    /// User-typed api-key replacement. Empty + `!api_key_edited` means "keep
    /// original"; empty + `api_key_edited` means "clear".
    pub(crate) api_key_input: String,
    pub(crate) api_key_original: String,
    pub(crate) api_key_edited: bool,
    pub reasoning: Reasoning,
    pub interleaved_thinking: bool,
    pub threshold: u64,
    pub fps: u32,
    pub capabilities_browser: bool,
    pub capabilities_computer_use: bool,
    pub focus: Field,
    pub error: Option<String>,
}

impl ModelMenu {
    pub fn new(config: &Config) -> Self {
        let original_key = config.provider.api_key.clone().unwrap_or_default();
        ModelMenu {
            model: config.model.clone(),
            base_url: config.provider.base_url.clone(),
            api_key_input: String::new(),
            api_key_original: original_key,
            api_key_edited: false,
            reasoning: Reasoning::from_config(config.reasoning_effort.as_deref()),
            interleaved_thinking: config.interleaved_thinking.unwrap_or(true),
            threshold: config.compaction.context_threshold,
            fps: config.tui_fps(),
            capabilities_browser: config.capabilities.browser,
            capabilities_computer_use: config.capabilities.computer_use,
            focus: Field::Model,
            error: None,
        }
    }

    /// What to display for the api-key field: masked original when untouched,
    /// asterisks for typed characters when editing.
    pub(crate) fn api_key_display(&self) -> String {
        if self.api_key_edited {
            "*".repeat(self.api_key_input.chars().count())
        } else {
            mask_key(&self.api_key_original)
        }
    }

    /// Resolve the api-key value to persist on save.
    pub(crate) fn resolve_api_key(&self) -> Option<String> {
        if self.api_key_edited {
            Some(self.api_key_input.clone())
        } else {
            None
        }
    }

    fn toggle_reasoning(&mut self) {
        self.reasoning = self.reasoning.next();
    }

    fn adjust_threshold(&mut self, delta: i64) {
        let next = self.threshold as i64 + delta;
        self.threshold = next.max(1000) as u64;
    }

    fn adjust_fps(&mut self, delta: i32) {
        self.fps = (self.fps as i32 + delta).clamp(1, 30) as u32;
    }

    pub(crate) fn build_patch(&self) -> ModelPatch {
        ModelPatch {
            model: self.model.clone(),
            base_url: self.base_url.clone(),
            api_key: self.resolve_api_key(),
            reasoning_effort: self.reasoning.to_option(),
            interleaved_thinking: Some(self.interleaved_thinking),
            context_threshold: self.threshold,
            fps: self.fps,
            capabilities_browser: self.capabilities_browser,
            capabilities_computer_use: self.capabilities_computer_use,
        }
    }
}

/// Handle one keystroke against an open `/config` modal.
pub fn handle_model_key(menu: &mut Option<ModelMenu>, k: KeyEvent) -> ModelOutcome {
    let m = match menu.as_mut() {
        Some(m) => m,
        None => return ModelOutcome::Idle,
    };
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        if matches!(
            k.code,
            KeyCode::Char('d') | KeyCode::Char('\u{4}')
        ) {
            *menu = None;
            return ModelOutcome::Quit;
        }
        return ModelOutcome::Idle;
    }
    m.error = None;
    match k.code {
        KeyCode::Esc => {
            *menu = None;
            ModelOutcome::Cancel
        }
        KeyCode::Tab => {
            m.focus = m.focus.next();
            ModelOutcome::Idle
        }
        KeyCode::BackTab => {
            m.focus = m.focus.prev();
            ModelOutcome::Idle
        }
        KeyCode::Up => {
            m.focus = m.focus.prev();
            ModelOutcome::Idle
        }
        KeyCode::Down => {
            m.focus = m.focus.next();
            ModelOutcome::Idle
        }
        KeyCode::Left => {
            match m.focus {
                Field::Reasoning => m.reasoning = m.reasoning.prev(),
                Field::InterleavedThinking => m.interleaved_thinking = !m.interleaved_thinking,
                Field::Browser => m.capabilities_browser = !m.capabilities_browser,
                Field::ComputerUse => m.capabilities_computer_use = !m.capabilities_computer_use,
                Field::Threshold => m.adjust_threshold(-1000),
                Field::Fps => m.adjust_fps(-1),
                _ => {}
            }
            ModelOutcome::Idle
        }
        KeyCode::Right => {
            match m.focus {
                Field::Reasoning => m.toggle_reasoning(),
                Field::InterleavedThinking => m.interleaved_thinking = !m.interleaved_thinking,
                Field::Browser => m.capabilities_browser = !m.capabilities_browser,
                Field::ComputerUse => m.capabilities_computer_use = !m.capabilities_computer_use,
                Field::Threshold => m.adjust_threshold(1000),
                Field::Fps => m.adjust_fps(1),
                _ => {}
            }
            ModelOutcome::Idle
        }
        KeyCode::Enter => match m.focus {
            Field::Save => {
                if let Err(e) = validate(m) {
                    m.error = Some(e);
                    return ModelOutcome::Idle;
                }
                let patch = m.build_patch();
                *menu = None;
                ModelOutcome::Save(patch)
            }
            Field::Cancel => {
                *menu = None;
                ModelOutcome::Cancel
            }
            // Confirm the current value and advance to the next field, so the
            // natural flow is: type → Enter → next field → … → Enter on [Save]
            // commits. Value changes for Reasoning/Threshold stay on
            // ↑/↓/←/→/Space, leaving Enter free as the confirm-and-advance key.
            _ => {
                m.focus = m.focus.next();
                ModelOutcome::Idle
            }
        },
        KeyCode::Backspace => match m.focus {
            Field::Model | Field::BaseUrl | Field::ApiKey => {
                edit_backspace(m);
                ModelOutcome::Idle
            }
            _ => ModelOutcome::Idle,
        },
        KeyCode::Char(c) => {
            match m.focus {
                Field::Model => m.model.push(c),
                Field::BaseUrl => m.base_url.push(c),
                Field::ApiKey => {
                    if !m.api_key_edited {
                        m.api_key_input.clear();
                        m.api_key_edited = true;
                    }
                    m.api_key_input.push(c);
                }
                Field::Threshold => {
                    if c.is_ascii_digit() {
                        let mut s = m.threshold.to_string();
                        if s == "0" {
                            s.clear();
                        }
                        s.push(c);
                        if let Ok(n) = s.parse::<u64>() {
                            m.threshold = n;
                        }
                    }
                }
                Field::Fps => {
                    if c.is_ascii_digit() {
                        let mut s = m.fps.to_string();
                        if s == "0" {
                            s.clear();
                        }
                        s.push(c);
                        if let Ok(n) = s.parse::<u32>() {
                            m.fps = n.clamp(1, 30);
                        }
                    }
                }
                Field::Reasoning if c == ' ' => m.toggle_reasoning(),
                Field::InterleavedThinking if c == ' ' => {
                    m.interleaved_thinking = !m.interleaved_thinking
                }
                Field::Browser if c == ' ' => m.capabilities_browser = !m.capabilities_browser,
                Field::ComputerUse if c == ' ' => {
                    m.capabilities_computer_use = !m.capabilities_computer_use
                }
                _ => {}
            }
            ModelOutcome::Idle
        }
        _ => ModelOutcome::Idle,
    }
}

fn edit_backspace(m: &mut ModelMenu) {
    match m.focus {
        Field::Model => {
            m.model.pop();
        }
        Field::BaseUrl => {
            m.base_url.pop();
        }
        Field::ApiKey => {
            if !m.api_key_edited {
                m.api_key_input.clear();
                m.api_key_edited = true;
            }
            m.api_key_input.pop();
        }
        _ => {}
    }
}

fn validate(m: &ModelMenu) -> std::result::Result<(), String> {
    if m.model.trim().is_empty() {
        return Err("model must not be empty".into());
    }
    if m.base_url.trim().is_empty() {
        return Err("base_url must not be empty".into());
    }
    if m.threshold < 1000 {
        return Err("context_threshold must be \u{2265} 1000".into());
    }
    Ok(())
}

/// `sk-****1234` style mask: keep the first 2 and last 4 chars when long
/// enough; otherwise show `****` to avoid leaking a short key verbatim.
pub fn mask_key(key: &str) -> String {
    let n = key.chars().count();
    if n == 0 {
        return "(unset)".to_string();
    }
    if n <= 6 {
        return "****".to_string();
    }
    let head: String = key.chars().take(2).collect();
    let tail: String = key.chars().skip(n.saturating_sub(4)).collect();
    format!("{head}****{tail}")
}

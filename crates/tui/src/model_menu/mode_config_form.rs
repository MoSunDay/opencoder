//! Act/Plan generation form with explicit overrides and Shared inheritance.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opencoder_core::{ConfigMode, ModeConfig};

use super::config_form::Reasoning;
use super::mode_select::ModeSelection;
use super::state::{ModelMenu, ModelOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeField {
    Reasoning,
    InterleavedThinking,
    MaxTokens,
    ContextLimit,
    CacheSalt,
    Save,
    Cancel,
}

impl ModeField {
    const ORDER: [Self; 7] = [
        Self::Reasoning,
        Self::InterleavedThinking,
        Self::MaxTokens,
        Self::ContextLimit,
        Self::CacheSalt,
        Self::Save,
        Self::Cancel,
    ];

    fn next(self) -> Self {
        let index = Self::ORDER
            .iter()
            .position(|field| *field == self)
            .unwrap_or(0);
        Self::ORDER[(index + 1) % Self::ORDER.len()]
    }

    fn prev(self) -> Self {
        let index = Self::ORDER
            .iter()
            .position(|field| *field == self)
            .unwrap_or(0);
        Self::ORDER[(index + Self::ORDER.len() - 1) % Self::ORDER.len()]
    }
}

pub struct ModeConfigForm {
    pub source: Box<ModeSelection>,
    pub mode: ConfigMode,
    pub focus: ModeField,
    pub reasoning: Reasoning,
    pub reasoning_inherit: bool,
    pub interleaved_thinking: Option<bool>,
    pub max_tokens_input: String,
    pub max_tokens_cursor: usize,
    pub context_limit_input: String,
    pub context_limit_cursor: usize,
    pub cache_salt: Option<bool>,
    pub error: Option<String>,
}

fn specific(source: &ModeSelection, mode: ConfigMode) -> ModeConfig {
    match mode {
        ConfigMode::Act => source.config.modes.act.clone().unwrap_or_default(),
        ConfigMode::Plan => source.config.modes.plan.clone().unwrap_or_default(),
        ConfigMode::Shared => source.config.modes.shared.clone(),
    }
}

impl ModeConfigForm {
    pub fn new(source: ModeSelection, mode: ConfigMode) -> Self {
        let specific = specific(&source, mode);
        let inherited = source.effective(ConfigMode::Shared);
        let max_tokens_input = specific
            .max_tokens
            .map(|value| value.to_string())
            .unwrap_or_default();
        let context_limit_input = specific
            .context_limit
            .map(|value| value.to_string())
            .unwrap_or_default();
        Self {
            reasoning: Reasoning::from_config(
                specific
                    .reasoning_effort
                    .as_deref()
                    .or(inherited.reasoning_effort.as_deref()),
            ),
            reasoning_inherit: specific.reasoning_effort.is_none(),
            interleaved_thinking: specific.interleaved_thinking,
            max_tokens_cursor: max_tokens_input.chars().count(),
            context_limit_cursor: context_limit_input.chars().count(),
            max_tokens_input,
            context_limit_input,
            cache_salt: specific.cache_salt,
            source: Box::new(source),
            mode,
            focus: ModeField::Reasoning,
            error: None,
        }
    }

    pub fn inherited(&self) -> opencoder_core::Config {
        self.source.effective(ConfigMode::Shared)
    }

    /// JSON null is intentional: Config's deep merge removes the override and
    /// restores Shared inheritance instead of copying the inherited value.
    pub fn build_patch(&self) -> serde_json::Value {
        let reasoning = if self.reasoning_inherit {
            serde_json::Value::Null
        } else {
            serde_json::json!(self.reasoning.to_option().unwrap_or_default())
        };
        let max_tokens = if self.max_tokens_input.trim().is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::json!(self.max_tokens_input.trim().parse::<u64>().unwrap_or(0))
        };
        let context_limit = if self.context_limit_input.trim().is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::json!(self.context_limit_input.trim().parse::<u64>().unwrap_or(0))
        };
        let key = match self.mode {
            ConfigMode::Act => "act",
            ConfigMode::Plan => "plan",
            ConfigMode::Shared => "shared",
        };
        serde_json::json!({
            "modes": {
                key: {
                    "reasoning_effort": reasoning,
                    "interleaved_thinking": self.interleaved_thinking,
                    "max_tokens": max_tokens,
                    "context_limit": context_limit,
                    "cache_salt": self.cache_salt,
                }
            }
        })
    }

    fn validate(&self) -> Result<(), String> {
        if !self.max_tokens_input.trim().is_empty()
            && self.max_tokens_input.trim().parse::<u64>().is_err()
        {
            return Err("max_tokens must be a number or empty (inherit)".into());
        }
        if !self.context_limit_input.trim().is_empty() {
            let value = self
                .context_limit_input
                .trim()
                .parse::<u64>()
                .map_err(|_| "context_limit must be a number or empty (inherit)")?;
            if value < 1_000 {
                return Err("context_limit override must be >= 1000".into());
            }
        }
        Ok(())
    }

    fn edit_number(&mut self, operation: impl FnOnce(&mut String, &mut usize)) {
        match self.focus {
            ModeField::MaxTokens => {
                operation(&mut self.max_tokens_input, &mut self.max_tokens_cursor)
            }
            ModeField::ContextLimit => operation(
                &mut self.context_limit_input,
                &mut self.context_limit_cursor,
            ),
            _ => {}
        }
    }

    fn clear_override(&mut self) {
        match self.focus {
            ModeField::Reasoning => self.reasoning_inherit = true,
            ModeField::InterleavedThinking => self.interleaved_thinking = None,
            ModeField::MaxTokens => {
                self.max_tokens_input.clear();
                self.max_tokens_cursor = 0;
            }
            ModeField::ContextLimit => {
                self.context_limit_input.clear();
                self.context_limit_cursor = 0;
            }
            ModeField::CacheSalt => self.cache_salt = None,
            ModeField::Save | ModeField::Cancel => {}
        }
    }

    pub fn paste_into(&mut self, text: &str) {
        let digits: String = text.chars().filter(|ch| ch.is_ascii_digit()).collect();
        self.edit_number(|input, cursor| {
            let (next, next_cursor) = crate::composer::insert_str(input, *cursor, &digits);
            *input = next;
            *cursor = next_cursor;
        });
    }
}

fn cycle_bool(value: &mut Option<bool>) {
    *value = match *value {
        None => Some(true),
        Some(true) => Some(false),
        Some(false) => None,
    };
}

fn back_to_scope(form: ModeConfigForm) -> (ModelOutcome, Option<ModelMenu>) {
    (ModelOutcome::Idle, Some(ModelMenu::ModeSelect(form.source)))
}

pub fn handle_key(mut form: ModeConfigForm, key: KeyEvent) -> (ModelOutcome, Option<ModelMenu>) {
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('l') | KeyCode::Char('u'))
    {
        form.clear_override();
        return (ModelOutcome::Idle, Some(ModelMenu::ModeConfig(form)));
    }
    match key.code {
        KeyCode::Esc => return back_to_scope(form),
        KeyCode::Up => form.focus = form.focus.prev(),
        KeyCode::Down | KeyCode::Tab => form.focus = form.focus.next(),
        KeyCode::BackTab => form.focus = form.focus.prev(),
        KeyCode::Delete => form.clear_override(),
        KeyCode::Enter => match form.focus {
            ModeField::Save => {
                if let Err(error) = form.validate() {
                    form.error = Some(error);
                } else {
                    return (ModelOutcome::Save(form.build_patch()), None);
                }
            }
            ModeField::Cancel => return back_to_scope(form),
            _ => form.focus = form.focus.next(),
        },
        KeyCode::Backspace => form.edit_number(|input, cursor| {
            if let Some((next, next_cursor)) = crate::composer::backspace(input, *cursor) {
                *input = next;
                *cursor = next_cursor;
            }
        }),
        KeyCode::Left => form.edit_number(|input, cursor| {
            *cursor = cursor.saturating_sub(1).min(input.chars().count())
        }),
        KeyCode::Right => {
            form.edit_number(|input, cursor| *cursor = (*cursor + 1).min(input.chars().count()))
        }
        KeyCode::Char('i') => form.clear_override(),
        KeyCode::Char(' ') => match form.focus {
            ModeField::Reasoning => {
                form.reasoning_inherit = false;
                form.reasoning = form.reasoning.next();
            }
            ModeField::InterleavedThinking => cycle_bool(&mut form.interleaved_thinking),
            ModeField::CacheSalt => cycle_bool(&mut form.cache_salt),
            _ => {}
        },
        KeyCode::Char(ch) if ch.is_ascii_digit() => form.edit_number(|input, cursor| {
            let (next, next_cursor) = crate::composer::insert_char(input, *cursor, ch);
            *input = next;
            *cursor = next_cursor;
        }),
        _ => {}
    }
    (ModelOutcome::Idle, Some(ModelMenu::ModeConfig(form)))
}

//! Shared/Act/Plan model-generation overlays.
//!
//! Top-level model fields remain the legacy base. `modes.shared` overlays that
//! base for every mode, and the optional Act/Plan blocks overlay Shared. All
//! resolution is performed on a cloned [`Config`], so callers can use the
//! existing endpoint, provider, and generation helpers without global state.

use serde::{Deserialize, Serialize};

use super::Config;

/// Selects the model-generation view exposed to a session or the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConfigMode {
    Shared,
    Act,
    Plan,
}

/// Optional model-generation values for one entry under `modes`.
///
/// `None` means "inherit": Act/Plan inherit Shared, and Shared inherits the
/// legacy top-level Config fields. Explicit JSON nulls remove a field while
/// deep-merging a patch, restoring that same inheritance behavior.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModeConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interleaved_thinking: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_salt: Option<bool>,
}

impl ModeConfig {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// Persisted `modes` object. The Shared block itself is optional on disk.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModesConfig {
    #[serde(default, skip_serializing_if = "ModeConfig::is_empty")]
    pub shared: ModeConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub act: Option<ModeConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<ModeConfig>,
}

impl ModesConfig {
    /// Keeps legacy serialization byte-for-byte free of a new empty `modes` key.
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

fn apply(base: &mut Config, overlay: &ModeConfig) {
    if let Some(model) = &overlay.model {
        base.model = model.clone();
    }
    if let Some(reasoning_effort) = &overlay.reasoning_effort {
        base.reasoning_effort = Some(reasoning_effort.clone());
    }
    if let Some(interleaved_thinking) = overlay.interleaved_thinking {
        base.interleaved_thinking = Some(interleaved_thinking);
    }
    if let Some(max_tokens) = overlay.max_tokens {
        base.max_tokens = Some(max_tokens);
    }
    if let Some(context_limit) = overlay.context_limit {
        base.context_limit = Some(context_limit);
    }
    if let Some(cache_salt) = overlay.cache_salt {
        base.cache_salt = Some(cache_salt);
    }
}

impl Config {
    /// Apply an explicit process/CLI model override to every mode.
    ///
    /// Only the legacy global model and the three optional mode model fields
    /// change. Generation overlays and provider settings remain intact, and a
    /// later interactive mode selection can still create a new override.
    /// Returns whether any model field changed.
    pub fn apply_explicit_model(&mut self, model: &str) -> bool {
        let changed = self.model != model
            || self.modes.shared.model.is_some()
            || self
                .modes
                .act
                .as_ref()
                .is_some_and(|mode| mode.model.is_some())
            || self
                .modes
                .plan
                .as_ref()
                .is_some_and(|mode| mode.model.is_some());
        self.model = model.to_string();
        self.modes.shared.model = None;
        if let Some(mode) = &mut self.modes.act {
            mode.model = None;
        }
        if let Some(mode) = &mut self.modes.plan {
            mode.model = None;
        }
        changed
    }

    /// Resolve a pure, independent Config view for `mode`.
    ///
    /// With no `modes` configuration this returns an exact clone, preserving
    /// legacy model/provider selection and every generation default. Provider
    /// endpoints continue to resolve from the effective `model` prefix.
    pub fn effective_for_mode(&self, mode: ConfigMode) -> Config {
        let mut effective = self.clone();
        apply(&mut effective, &self.modes.shared);
        let specific = match mode {
            ConfigMode::Shared => None,
            ConfigMode::Act => self.modes.act.as_ref(),
            ConfigMode::Plan => self.modes.plan.as_ref(),
        };
        if let Some(specific) = specific {
            apply(&mut effective, specific);
        }
        effective
    }
}

//! User-registered CLI instructions injected into the model system prompt.

use crate::AgentMode;
use serde::{Deserialize, Serialize};

/// Which agent tier receives a registered integration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum InjectionTarget {
    #[default]
    Parent,
    Subagents,
    All,
}

impl InjectionTarget {
    pub fn allows(self, mode: AgentMode) -> bool {
        matches!(self, Self::All)
            || matches!(
                (self, mode),
                (Self::Parent, AgentMode::Primary) | (Self::Subagents, AgentMode::Subagent)
            )
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Parent => "parent",
            Self::Subagents => "subagents",
            Self::All => "all",
        }
    }

    pub fn is_parent(&self) -> bool {
        matches!(self, Self::Parent)
    }

    pub fn next(self) -> Self {
        match self {
            Self::Parent => Self::Subagents,
            Self::Subagents => Self::All,
            Self::All => Self::Parent,
        }
    }
}

/// One named CLI registration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CliConfig {
    /// Disabled registrations remain editable but are not sent to the model.
    #[serde(default)]
    pub enabled: bool,
    /// Agent tier that receives this registration.
    #[serde(default, skip_serializing_if = "InjectionTarget::is_parent")]
    pub inject_to: InjectionTarget,
    /// Free-form usage contract for this CLI (commands, constraints, examples).
    #[serde(default)]
    pub content: String,
}

/// Merge a JSON patch into a CLI registration without clearing omitted fields.
pub(super) fn merge(cfg: &mut CliConfig, obj: &serde_json::Map<String, serde_json::Value>) {
    if let Some(enabled) = obj.get("enabled").and_then(|v| v.as_bool()) {
        cfg.enabled = enabled;
    }
    if let Some(target) = obj.get("inject_to").and_then(|v| v.as_str()) {
        if let Ok(target) = serde_json::from_value(serde_json::Value::String(target.to_string())) {
            cfg.inject_to = target;
        }
    }
    if let Some(content) = obj.get("content").and_then(|v| v.as_str()) {
        cfg.content = content.to_string();
    }
}

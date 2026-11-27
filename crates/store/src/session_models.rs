use serde::{Deserialize, Serialize};

/// Session-only model selections for Shared and the built-in Act/Plan modes.
///
/// The containing `Option` in storage APIs is significant: `None` denotes a
/// legacy row and preserves its old single-model behavior. `Some(default())`
/// denotes a mode-aware row with every selection inheriting configuration.
/// Missing fields in older JSON objects, including `shared`, deserialize to
/// `None` and therefore retain configuration inheritance.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionModelOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub act: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
}

impl SessionModelOverrides {
    pub fn get(&self, mode: opencoder_core::ConfigMode) -> Option<&str> {
        match mode {
            opencoder_core::ConfigMode::Act => self.act.as_deref(),
            opencoder_core::ConfigMode::Plan => self.plan.as_deref(),
            opencoder_core::ConfigMode::Shared => self.shared.as_deref(),
        }
    }

    pub fn set(&mut self, mode: opencoder_core::ConfigMode, model: Option<String>) -> bool {
        let target = match mode {
            opencoder_core::ConfigMode::Act => &mut self.act,
            opencoder_core::ConfigMode::Plan => &mut self.plan,
            opencoder_core::ConfigMode::Shared => &mut self.shared,
        };
        *target = model;
        true
    }
}

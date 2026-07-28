use serde::{Deserialize, Serialize};

/// Autopilot loop configuration. When enabled, the session runner cycles
/// PLAN -> ACT -> VERIFY after the initial task, where VERIFY is an isolated
/// shadow one-shot that judges whether more work is needed. See the
/// `autopilot` module in `opencoder-session`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoPilotConfig {
    /// Master switch. Off by default so ordinary sessions behave classically.
    #[serde(default)]
    pub enabled: bool,
    /// Hard cap on PLAN->ACT->VERIFY iterations (0-based count of completed
    /// cycles). Reaching it yields `ApOutcome::MaxIterations`.
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    /// Optional skill name to activate for the PLAN phase. Resolved from the
    /// discovered skills at run time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    /// How many times to retry a malformed VERIFY verdict before aborting.
    #[serde(default = "default_verify_retries")]
    pub verify_retries: u32,
}

fn default_max_iterations() -> u32 {
    10
}
fn default_verify_retries() -> u32 {
    3
}

impl Default for AutoPilotConfig {
    fn default() -> Self {
        AutoPilotConfig {
            enabled: false,
            max_iterations: default_max_iterations(),
            skill: None,
            verify_retries: default_verify_retries(),
        }
    }
}

/// Merge autopilot-specific keys from `obj` (the inner `"autopilot"` object)
/// into `cfg`. The caller is responsible for selecting the `autopilot` key.
pub(super) fn merge(cfg: &mut AutoPilotConfig, obj: &serde_json::Map<String, serde_json::Value>) {
    if let Some(v) = obj.get("enabled").and_then(|v| v.as_bool()) {
        cfg.enabled = v;
    }
    if let Some(v) = obj.get("max_iterations").and_then(|v| v.as_u64()) {
        cfg.max_iterations = v as u32;
    }
    if let Some(v) = obj.get("verify_retries").and_then(|v| v.as_u64()) {
        cfg.verify_retries = v as u32;
    }
    if let Some(v) = obj.get("skill").and_then(|v| v.as_str()) {
        let t = v.trim();
        cfg.skill = if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        };
    }
}

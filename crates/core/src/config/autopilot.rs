use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoPilotConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
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
            verify_retries: default_verify_retries(),
        }
    }
}

pub(super) fn merge(cfg: &mut AutoPilotConfig, obj: &serde_json::Map<String, serde_json::Value>) {
    if let Some(v) = obj.get("enabled").and_then(|v| v.as_bool()) {
        cfg.enabled = v;
    }
    if let Some(v) = obj.get("max_iterations").and_then(|v| v.as_u64()) {
        cfg.max_iterations = v.min(u32::MAX as u64) as u32;
    }
    if let Some(v) = obj.get("verify_retries").and_then(|v| v.as_u64()) {
        cfg.verify_retries = v.min(u32::MAX as u64) as u32;
    }
}

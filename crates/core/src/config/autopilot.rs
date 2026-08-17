use serde::{Deserialize, Serialize};

/// Autopilot operating mode (three-state, default `Off`).
///
/// - `Off` — no automatic behavior after the initial task.
/// - `Ap` — the fully automatic PLAN → ACT → VERIFY self-driving loop.
/// - `Review` — one-shot automatic review (plan agent + review skill) after
///   the initial task, then stop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApMode {
    #[default]
    Off,
    Ap,
    Review,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoPilotConfig {
    #[serde(default)]
    pub mode: ApMode,
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
            mode: ApMode::Off,
            max_iterations: default_max_iterations(),
            verify_retries: default_verify_retries(),
        }
    }
}

/// Parse a `mode` string from raw JSON; unknown values are ignored (the
/// caller keeps the previous value), matching the lenient scalar handling
/// used for every other merge key.
fn parse_mode(v: &str) -> Option<ApMode> {
    match v {
        "off" => Some(ApMode::Off),
        "ap" => Some(ApMode::Ap),
        "review" => Some(ApMode::Review),
        _ => None,
    }
}

pub(super) fn merge(cfg: &mut AutoPilotConfig, obj: &serde_json::Map<String, serde_json::Value>) {
    // `mode` is the canonical key. Legacy configs carry only the boolean
    // `enabled`: `true` maps to `ap` (so old users keep their self-driving
    // loop instead of being silently switched off), `false` maps to `off`.
    // When both keys are present `mode` wins and `enabled` is ignored.
    if let Some(v) = obj
        .get("mode")
        .and_then(|v| v.as_str())
        .and_then(parse_mode)
    {
        cfg.mode = v;
    } else {
        // `mode` present but unrecognized (typo, wrong type): surface it —
        // a silently ignored mode means the user thinks autopilot is on
        // while it stays off. The lenient fallback chain is unchanged.
        if let Some(raw) = obj.get("mode") {
            tracing::warn!(
                value = ?raw,
                "unrecognized config autopilot.mode (expected \"off\"|\"ap\"|\"review\"); ignoring"
            );
        }
        if let Some(v) = obj.get("enabled").and_then(|v| v.as_bool()) {
            cfg.mode = if v { ApMode::Ap } else { ApMode::Off };
        }
    }
    if let Some(v) = obj.get("max_iterations").and_then(|v| v.as_u64()) {
        cfg.max_iterations = v.min(u32::MAX as u64) as u32;
    }
    if let Some(v) = obj.get("verify_retries").and_then(|v| v.as_u64()) {
        cfg.verify_retries = v.min(u32::MAX as u64) as u32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn merged(json: serde_json::Value) -> AutoPilotConfig {
        let mut cfg = AutoPilotConfig::default();
        merge(&mut cfg, json.as_object().unwrap());
        cfg
    }

    #[test]
    fn mode_parses_all_three_states_and_ignores_unknown() {
        assert_eq!(merged(json!({"mode": "ap"})).mode, ApMode::Ap);
        assert_eq!(merged(json!({"mode": "review"})).mode, ApMode::Review);
        assert_eq!(merged(json!({"mode": "off"})).mode, ApMode::Off);
        // Unknown string keeps the previous value (default off).
        assert_eq!(
            merged(json!({"mode": "warp"})).mode,
            ApMode::Off,
            "unknown mode ignored"
        );
    }

    /// An unrecognized `mode` keeps the lenient fallback chain: a legacy
    /// `enabled` key present alongside it still applies (the merge only
    /// warns — it must not silently strand a user who typo'd the mode but
    /// still carries the legacy boolean).
    #[test]
    fn unknown_mode_warns_but_keeps_legacy_fallback() {
        assert_eq!(
            merged(json!({"mode": "warp", "enabled": true})).mode,
            ApMode::Ap,
            "bad mode + legacy enabled=true still maps to ap"
        );
        assert_eq!(
            merged(json!({"mode": 3, "enabled": false})).mode,
            ApMode::Off,
            "non-string mode falls back to legacy enabled=false"
        );
    }

    #[test]
    fn legacy_enabled_maps_ap_and_off() {
        assert_eq!(merged(json!({"enabled": true})).mode, ApMode::Ap);
        assert_eq!(merged(json!({"enabled": false})).mode, ApMode::Off);
    }

    #[test]
    fn mode_wins_over_legacy_enabled_when_both_present() {
        assert_eq!(
            merged(json!({"mode": "off", "enabled": true})).mode,
            ApMode::Off,
            "mode is canonical"
        );
    }
}

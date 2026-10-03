//! Skill default-injection toggles (`/skill` menu). Entries mirror skills
//! discovered under `~/.opencoder/skills`; `enabled == true` names are listed
//! in a transient tail reminder appended to the LLM context by the session
//! runtime. Content lives on disk (SKILL.md) — config never stores bodies.

use serde::{Deserialize, Serialize};

/// Default-injection toggle for one named skill. Default OFF.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillConfig {
    /// When `true` the skill's name is included in the context-tail skill
    /// catalog reminder at LLM-call time.
    #[serde(default)]
    pub enabled: bool,
}

/// Merge a JSON object patch into a single `SkillConfig` entry, field by
/// field (siblings preserved, mirroring the `cli` merge pattern).
pub(super) fn merge(dst: &mut SkillConfig, obj: &serde_json::Map<String, serde_json::Value>) {
    if let Some(b) = obj.get("enabled").and_then(|v| v.as_bool()) {
        dst.enabled = b;
    }
}

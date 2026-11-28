use opencoder_core::looks_like_env_var;

#[derive(Debug, Clone)]
pub struct ConfigPatch {
    pub reasoning_effort: Option<String>,
    pub interleaved_thinking: Option<bool>,
    pub max_tokens: Option<u64>,
    pub context_threshold: u64,
    pub context_limit: u64,
    pub fps: u32,
    pub enable_tmux_session: Option<bool>,
    pub subagent: bool,
}

impl ConfigPatch {
    pub fn to_json(&self) -> serde_json::Value {
        let mut root = serde_json::json!({
            // Off serializes to "" so the key persists on disk.
            // (merge_json treats null as delete, which would drop the key.)
            "reasoning_effort": self.reasoning_effort.clone().unwrap_or_default(),
            "context_limit": self.context_limit,
            "compaction": { "context_threshold": self.context_threshold },
            "fps": self.fps,
            "subagent": self.subagent,
        });
        if let Some(mt) = self.max_tokens {
            root["max_tokens"] = serde_json::json!(mt);
        }
        if let Some(it) = self.interleaved_thinking {
            root["interleaved_thinking"] = serde_json::json!(it);
        }
        if let Some(ets) = self.enable_tmux_session {
            root["enable_tmux_session"] = serde_json::json!(ets);
        }
        root
    }
}

#[derive(Debug, Clone)]
pub struct ProviderPatch {
    pub protocol: String,
    pub name: String,
    pub model_id: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub headers: Vec<(String, String)>,
}

impl ProviderPatch {
    pub fn to_json(&self) -> serde_json::Value {
        let mut provider =
            serde_json::json!({ "base_url": self.base_url, "protocol": self.protocol });
        provider["model"] = serde_json::Value::String(self.model_id.clone());
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
        provider["headers"] = serde_json::Value::Array(
            self.headers
                .iter()
                .map(|(n, v)| serde_json::json!({ "name": n, "value": v }))
                .collect(),
        );
        let mut providers = serde_json::Map::new();
        providers.insert(self.name.clone(), provider);
        serde_json::json!({ "model": format!("{}/{}", self.name, self.model_id), "providers": serde_json::Value::Object(providers) })
    }

    /// `/model` only ever reaches this with Act or Plan: the edit selects the
    /// provider for that mode and never replaces the Shared default or the
    /// other mode's model. The Shared branch is kept for first-run
    /// onboarding (`ProviderForm::new_onboarding`), whose whole purpose is to
    /// write the configuration file's top-level default.
    pub fn to_json_for_mode(&self, mode: opencoder_core::ConfigMode) -> serde_json::Value {
        let mut patch = self.to_json();
        if mode != opencoder_core::ConfigMode::Shared {
            if let Some(root) = patch.as_object_mut() {
                root.remove("model");
            }
        }
        patch
    }
}

pub fn delete_provider_json(name: &str) -> serde_json::Value {
    serde_json::json!({ "providers": { name: serde_json::Value::Null } })
}

/// Merge-patch selecting `name/model_id` for one `/model` scope. `/model`
/// has no Shared scope, so a Shared request normalizes to Act (the same
/// rule as `ModeMenuKind::Model::normalize_mode`).
pub fn switch_mode_provider_json(
    mode: opencoder_core::ConfigMode,
    name: &str,
    model_id: &str,
) -> serde_json::Value {
    let model = format!("{}/{}", name, model_id);
    match mode {
        opencoder_core::ConfigMode::Shared | opencoder_core::ConfigMode::Act => {
            serde_json::json!({ "modes": { "act": { "model": model } } })
        }
        opencoder_core::ConfigMode::Plan => {
            serde_json::json!({ "modes": { "plan": { "model": model } } })
        }
    }
}

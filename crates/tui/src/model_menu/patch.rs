//! Patch types produced by the `/config` and `/model` forms. Each builds the
//! JSON merge-patch consumed by `Config::save`.

use opencoder_core::looks_like_env_var;

/// Generation-parameter edits from the slim `/config` form. Carries only the
/// fields `/config` owns (no model/base_url/api_key — those live in `/model`).
#[derive(Debug, Clone)]
pub struct ConfigPatch {
    pub reasoning_effort: Option<String>,
    pub interleaved_thinking: Option<bool>,
    /// `None` = omit from patch (leave existing); `Some(v)` = write `v`.
    pub max_tokens: Option<u64>,
    pub context_threshold: u64,
    pub fps: u32,
    pub capabilities_browser: bool,
    pub capabilities_computer_use: bool,
    pub capabilities_tools_subagent: bool,
}

impl ConfigPatch {
    pub fn to_json(&self) -> serde_json::Value {
        let mut root = serde_json::json!({
            "reasoning_effort": self.reasoning_effort,
            "interleaved_thinking": self.interleaved_thinking,
            "compaction": {
                "context_threshold": self.context_threshold,
            },
            "fps": self.fps,
            "capabilities": {
                "browser": self.capabilities_browser,
                "computer_use": self.capabilities_computer_use,
                "tools_subagent": self.capabilities_tools_subagent,
            },
        });
        if let Some(mt) = self.max_tokens {
            root["max_tokens"] = serde_json::json!(mt);
        }
        root
    }
}

/// Provider-identity edits from the `/model` form. Writes `providers[name]`
/// and sets `model = "{name}/{model_id}"`.
///
/// `api_key: None` means "leave the existing value untouched" (omitted from
/// the provider object in the patch so `merge_json` preserves it); `Some("")`
/// means explicit clear (`null` in patch → merge removes); `Some(v)` writes
/// the value, wrapping env-var-shaped names as `{NAME}`.
#[derive(Debug, Clone)]
pub struct ProviderPatch {
    pub name: String,
    pub model_id: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub headers: Vec<(String, String)>,
}

impl ProviderPatch {
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
        provider["headers"] = serde_json::Value::Array(
            self.headers
                .iter()
                .map(|(n, v)| {
                    serde_json::json!({ "name": n, "value": v })
                })
                .collect(),
        );
        let mut providers = serde_json::Map::new();
        providers.insert(self.name.clone(), provider);
        serde_json::json!({
            "model": format!("{}/{}", self.name, self.model_id),
            "providers": serde_json::Value::Object(providers),
        })
    }
}

/// Build the JSON merge-patch to delete a named provider: sets
/// `providers[name]` to `null` so `merge_json` removes the key.
pub fn delete_provider_json(name: &str) -> serde_json::Value {
    serde_json::json!({
        "providers": {
            name: serde_json::Value::Null,
        }
    })
}

/// Build the JSON merge-patch to switch the active model to
/// `"{name}/{model_id}"` without modifying provider entries.
pub fn switch_provider_json(name: &str, model_id: &str) -> serde_json::Value {
    serde_json::json!({
        "model": format!("{}/{}", name, model_id),
    })
}

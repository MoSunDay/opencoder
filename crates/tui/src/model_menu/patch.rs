use opencoder_core::looks_like_env_var;

#[derive(Debug, Clone)]
pub struct ConfigPatch {
    pub reasoning_effort: Option<String>,
    pub interleaved_thinking: Option<bool>,
    pub max_tokens: Option<u64>,
    pub context_threshold: u64,
    pub context_limit: u64,
    pub fps: u32,
    pub capabilities_browser: bool,
    pub capabilities_computer_use: bool,
    pub capabilities_tools_subagent: bool,
    pub ap_enabled: bool,
    pub ap_max_iter: u32,
}

impl ConfigPatch {
    pub fn to_json(&self) -> serde_json::Value {
        let mut root = serde_json::json!({
            "reasoning_effort": self.reasoning_effort,
            "context_limit": self.context_limit,
            "interleaved_thinking": self.interleaved_thinking,
            "compaction": { "context_threshold": self.context_threshold },
            "fps": self.fps,
            "capabilities": {
                "browser": self.capabilities_browser,
                "computer_use": self.capabilities_computer_use,
                "tools_subagent": self.capabilities_tools_subagent,
            },
            "autopilot": {
                "enabled": self.ap_enabled,
                "max_iterations": self.ap_max_iter,
            },
        });
        if let Some(mt) = self.max_tokens { root["max_tokens"] = serde_json::json!(mt); }
        root
    }
}

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
            self.headers.iter()
                .map(|(n, v)| serde_json::json!({ "name": n, "value": v }))
                .collect(),
        );
        let mut providers = serde_json::Map::new();
        providers.insert(self.name.clone(), provider);
        serde_json::json!({ "model": format!("{}/{}", self.name, self.model_id), "providers": serde_json::Value::Object(providers) })
    }
}

pub fn delete_provider_json(name: &str) -> serde_json::Value {
    serde_json::json!({ "providers": { name: serde_json::Value::Null } })
}

pub fn switch_provider_json(name: &str, model_id: &str) -> serde_json::Value {
    serde_json::json!({ "model": format!("{}/{}", name, model_id) })
}

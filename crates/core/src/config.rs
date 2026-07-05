use crate::error::{CoreError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub provider: ProviderConfig,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub small_model: Option<String>,
    #[serde(default)]
    pub agent: AgentDefaults,
    #[serde(default)]
    pub compaction: CompactionConfig,
    #[serde(default)]
    pub max_steps: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<u64>,
    /// Max output tokens per generation. When unset the provider default is
    /// used — but some providers (e.g. glm5.2) ship a small default that
    /// truncates large tool-call payloads mid-stream (`finish_reason=length`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
}

fn default_model() -> String {
    "openai/gpt-4o-mini".to_string()
}

/// Default context window assumed when neither config nor a model registry
/// supplies one. Large enough that the `context_threshold` is the binding
/// constraint by default, but lets `reserved` take effect once set.
pub const DEFAULT_CONTEXT_LIMIT: u64 = 128_000;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
}

fn default_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefaults {
    #[serde(default)]
    pub default: String,
}
impl Default for AgentDefaults {
    fn default() -> Self {
        AgentDefaults { default: "act".to_string() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    #[serde(default = "default_true")]
    pub auto: bool,
    #[serde(default = "default_threshold")]
    pub context_threshold: u64,
    #[serde(default = "default_tail_turns")]
    pub tail_turns: u32,
    #[serde(default = "default_reserved")]
    pub reserved: u64,
    #[serde(default)]
    pub prune: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buffer: Option<u64>,
}
impl Default for CompactionConfig {
    fn default() -> Self {
        CompactionConfig {
            auto: true,
            context_threshold: 80_000,
            tail_turns: 2,
            reserved: 20_000,
            prune: false,
            buffer: None,
        }
    }
}
fn default_true() -> bool { true }
fn default_threshold() -> u64 { 80_000 }
fn default_tail_turns() -> u32 { 2 }
fn default_reserved() -> u64 { 20_000 }

impl Default for Config {
    fn default() -> Self {
        Config {
            provider: ProviderConfig::default(),
            model: default_model(),
            small_model: None,
            agent: AgentDefaults::default(),
            compaction: CompactionConfig::default(),
            max_steps: 50,
            context_limit: None,
            max_tokens: None,
        }
    }
}

impl Config {
    pub fn load(working_dir: &Path) -> Result<Config> {
        let mut cfg = Config::default();
        // Merge ALL existing candidates, least-specific first so project files
        // override the global base (matches opencode). This lets ~/.opencoder
        // provide the provider+key while a project opencode.json overrides only
        // the model — `opencoder` then runs directly from any directory.
        let mut candidates = config_candidates(working_dir);
        candidates.reverse(); // global first, project last (wins)
        for p in candidates {
            if p.exists() {
                let raw = std::fs::read_to_string(&p)?;
                let parsed: serde_json::Value = serde_json::from_str(&raw)?;
                merge_into(&mut cfg, parsed);
            }
        }
        apply_env(&mut cfg);
        Ok(cfg)
    }
    pub fn model_id(&self) -> &str {
        self.model.split_once('/').map(|(_, m)| m).unwrap_or(&self.model)
    }
    pub fn provider_id(&self) -> &str {
        self.model.split_once('/').map(|(p, _)| p).unwrap_or("openai")
    }
    /// Effective context window: explicit override, else the default.
    pub fn context_limit(&self) -> u64 {
        self.context_limit.unwrap_or(DEFAULT_CONTEXT_LIMIT)
    }
    /// Model id used for low-cost background calls (title generation, compaction
    /// summarization). Returns the id (after the `/`) so the request body carries
    /// a bare model id matching the fixed `base_url` — the provider prefix must
    /// NOT be sent to the provider.
    pub fn small_model_id(&self) -> &str {
        match &self.small_model {
            Some(s) => s.split_once('/').map(|(_, m)| m).unwrap_or(s),
            None => self.model_id(),
        }
    }
    /// Bare model id for the background-call request body. Falls back to the
    /// primary model id when no small_model is configured.
    pub fn small_model_or_primary(&self) -> &str {
        self.small_model_id()
    }
    pub fn api_key(&self) -> Result<String> {
        self.provider
            .api_key
            .clone()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| CoreError::Config("missing OPENAI_API_KEY".into()))
    }
}

fn config_candidates(working_dir: &Path) -> Vec<PathBuf> {
    let mut v = vec![
        working_dir.join(".opencode").join("config.json"),
        working_dir.join("opencode.json"),
    ];
    if let Some(home) = dirs::home_dir() {
        // ~/.opencoder/ (this binary's own config home) — highest-priority global,
        // so `opencoder` runs directly from any directory with no project config.
        v.push(home.join(".opencoder").join("config.json"));
        v.push(home.join(".opencoder").join("opencode.json"));
        v.push(home.join(".opencode").join("config.json"));
    }
    if let Some(cfg) = dirs::config_dir() {
        v.push(cfg.join("opencode").join("config.json"));
    }
    v
}

fn apply_env(cfg: &mut Config) {
    if let Ok(b) = std::env::var("OPENAI_BASE_URL") {
        if !b.is_empty() {
            cfg.provider.base_url = b.trim_end_matches('/').to_string();
        }
    }
    if let Ok(m) = std::env::var("OPENCODE_MODEL") {
        if !m.is_empty() {
            cfg.model = m;
        }
    }
    if let Ok(m) = std::env::var("OPENCODE_SMALL_MODEL") {
        if !m.is_empty() {
            cfg.small_model = Some(m);
        }
    }
    if let Ok(v) = std::env::var("OPENCODE_CONTEXT_LIMIT") {
        if let Ok(n) = v.parse::<u64>() {
            cfg.context_limit = Some(n);
        }
    }
}

fn merge_into(cfg: &mut Config, value: serde_json::Value) {
    if let Some(obj) = value.as_object() {
        if let Some(model) = obj.get("model").and_then(|v| v.as_str()) {
            cfg.model = model.to_string();
        }
        if let Some(small) = obj.get("small_model").and_then(|v| v.as_str()) {
            cfg.small_model = Some(small.to_string());
        }
        if let Some(cl) = obj.get("context_limit").and_then(|v| v.as_u64()) {
            cfg.context_limit = Some(cl);
        }
        if let Some(mt) = obj.get("max_tokens").and_then(|v| v.as_u64()) {
            cfg.max_tokens = Some(mt);
        }
        if let Some(steps) = obj.get("max_steps").and_then(|v| v.as_u64()) {
            cfg.max_steps = steps as u32;
        }
        if let Some(p) = obj.get("provider").and_then(|v| v.as_object()) {
            if let Some(b) = p.get("base_url").and_then(|v| v.as_str()) {
                cfg.provider.base_url = b.to_string();
            }
            if let Some(k) = p.get("api_key").and_then(|v| v.as_str()) {
                cfg.provider.api_key = Some(resolve_env(k));
            }
        }
        if let Some(c) = obj.get("compaction").and_then(|v| v.as_object()) {
            if let Some(v) = c.get("auto").and_then(|v| v.as_bool()) { cfg.compaction.auto = v; }
            if let Some(v) = c.get("context_threshold").and_then(|v| v.as_u64()) { cfg.compaction.context_threshold = v; }
            if let Some(v) = c.get("tail_turns").and_then(|v| v.as_u64()) { cfg.compaction.tail_turns = v as u32; }
            if let Some(v) = c.get("reserved").and_then(|v| v.as_u64()) { cfg.compaction.reserved = v; }
            if let Some(v) = c.get("prune").and_then(|v| v.as_bool()) { cfg.compaction.prune = v; }
            if let Some(v) = c.get("buffer").and_then(|v| v.as_u64()) { cfg.compaction.buffer = Some(v); }
        }
        if let Some(a) = obj.get("agent").and_then(|v| v.as_object()) {
            if let Some(d) = a.get("default").and_then(|v| v.as_str()) { cfg.agent.default = d.to_string(); }
        }
    }
}

fn resolve_env(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        let name = &trimmed[1..trimmed.len() - 1];
        std::env::var(name).unwrap_or_default()
    } else {
        trimmed.to_string()
    }
}

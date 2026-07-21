use crate::error::{CoreError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub provider: ProviderConfig,
    /// Named OpenAI-compatible providers. Each entry is `{base_url, api_key?, model?}`.
    /// The active provider is selected by the `provider/` prefix of `model`.
    /// Empty by default; populate via config file. No built-in presets.
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub small_model: Option<String>,
    #[serde(default)]
    pub agent: AgentDefaults,
    #[serde(default)]
    pub compaction: CompactionConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<u64>,
    /// Max output tokens per generation. When unset the provider default is
    /// used — but some providers (e.g. glm5.2) ship a small default that
    /// truncates large tool-call payloads mid-stream (`finish_reason=length`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    /// OpenAI-style reasoning effort sent as a top-level `reasoning_effort`
    /// field on the chat request body. Accepted values: `low|medium|high`.
    /// When `None` the field is omitted (provider default / no extended
    /// thinking). Edited at runtime via the TUI `/model` menu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// Per-agent prefix-cache salting. `Some(true)` (the default) makes every
    /// outbound LLM request carry a top-level `cache_salt` body field equal to
    /// `<agent_name>:<session_id>`, so a vLLM / prefix-cache backend can
    /// namespace its KV cache per agent/conversation and grow the cached prefix
    /// across turns within a conversation. `Some(false)` or `None` omits the
    /// field entirely (no behavior change). The value is stable across an
    /// agent's turns; subagents derive their own salt from their child session
    /// id (`sub-<ULID>`), so each subagent run gets an independent namespace.
    #[serde(default = "default_cache_salt", skip_serializing_if = "Option::is_none")]
    pub cache_salt: Option<bool>,
    /// Interleaved thinking: when true, the `reasoning_content` produced on
    /// tool-call turns is persisted into the assistant message and sent back
    /// on subsequent requests, letting the model continue its chain-of-thought
    /// across tool results. Required by some providers (e.g. DeepSeek-V4
    /// returns HTTP 400 if reasoning_content is omitted after a tool call).
    /// Defaults to `Some(true)`.
    #[serde(
        default = "default_interleaved_thinking",
        skip_serializing_if = "is_none_interleaved"
    )]
    pub interleaved_thinking: Option<bool>,
    /// TUI render frame rate (FPS), clamped to 1..=30 at runtime. Higher
    /// values raise CPU usage; 10 is already smooth. `None` = default (10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<u32>,
    /// Outbound proxy for LLM + browser traffic. Accepts `socks5://`,
    /// `socks5h://`, `http://`, `https://`. The effective value also honors
    /// `OPENCODER_PROXY` / `ALL_PROXY` env vars (see `net::effective_proxy`).
    #[serde(default)]
    pub network: NetworkConfig,
    /// Capability toggles gating the optional browser + computer-use tools and
    /// the `tools` umbrella subagent. All three default to off.
    #[serde(default)]
    pub capabilities: CapabilitiesConfig,
}

fn default_interleaved_thinking() -> Option<bool> {
    Some(true)
}

fn default_cache_salt() -> Option<bool> {
    Some(true)
}

fn is_none_interleaved(v: &Option<bool>) -> bool {
    v.is_none()
}

/// Warn (without rewriting) when the configured `model` looks like a stale or
/// malformed value that would silently break requests. Only logs — never
/// mutates the user's config. Catches legacy values such as single-char or
/// placeholder strings so they are not silently written back to config.json.
/// Pure predicate: is the `model` string malformed (too short on either side
/// of the `/`, or too short when unscoped)? Exposed for unit testing.
pub(crate) fn is_suspicious_model(model: &str) -> bool {
    if model.is_empty() {
        return false;
    }
    match model.split_once('/') {
        Some((provider, mid)) => provider.len() < 2 || mid.len() < 2,
        None => model.len() < 3,
    }
}

pub(crate) fn warn_if_suspicious_model(model: &str) {
    if is_suspicious_model(model) {
        tracing::warn!(
            model = %model,
            "config `model` looks malformed (expected `provider/model`, e.g. `openai/gpt-4o`); override with --model if this is a stale value"
        );
    }
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
    /// Default model id for this provider (the part after the `/` prefix).
    #[serde(default)]
    pub model: Option<String>,
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
        AgentDefaults {
            default: "act".to_string(),
        }
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
            buffer: None,
        }
    }
}
fn default_true() -> bool {
    true
}
fn default_threshold() -> u64 {
    80_000
}
fn default_tail_turns() -> u32 {
    2
}
fn default_reserved() -> u64 {
    20_000
}

/// Networking options for outbound LLM + browser traffic.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkConfig {
    /// Proxy URL (`socks5://`, `socks5h://`, `http://`, `https://`). `None`
    /// means a direct connection (subject to env-var fallback at use time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
}

/// Capability switches. Each gates a family of optional tools so the model only
/// sees (and the registry only activates) capabilities the user has opted into.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CapabilitiesConfig {
    /// Enable `web_fetch` / `web_search` + the `tools` subagent's browser tools.
    /// Requires the `browser` cargo feature at compile time.
    #[serde(default)]
    pub browser: bool,
    /// Enable the `computer_use` tool + the `tools` subagent's computer-use tool.
    #[serde(default)]
    pub computer_use: bool,
    /// Enable the `tools` umbrella subagent (browser/computer-use delegation).
    /// When off, the system prompt drops the 'tools' advertisement, the task
    /// schema omits the 'tools' subagent_type, and `run_subagent` rejects
    /// `subagent_type: "tools"`.
    #[serde(default)]
    pub tools_subagent: bool,
}

impl CapabilitiesConfig {
    /// Whether a given tool name is enabled by the capability switches.
    /// Capability-gated tools (`web_fetch`/`web_search`, `computer_use`) return
    /// `false` unless their switch is on; every other tool is always enabled.
    pub fn tool_enabled(&self, name: &str) -> bool {
        match name {
            "web_fetch" | "web_search" => self.browser,
            "computer_use" => self.computer_use,
            _ => true,
        }
    }

    /// Whether the `tools` umbrella subagent is enabled. When false, the system
    /// prompt drops the 'tools' advertisement, the task schema omits the 'tools'
    /// subagent_type, and `run_subagent` rejects `subagent_type: "tools"`.
    pub fn tools_subagent_enabled(&self) -> bool {
        self.tools_subagent
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            provider: ProviderConfig::default(),
            providers: HashMap::new(),
            model: default_model(),
            small_model: None,
            agent: AgentDefaults::default(),
            compaction: CompactionConfig::default(),
            context_limit: None,
            max_tokens: None,
            reasoning_effort: None,
            cache_salt: default_cache_salt(),
            interleaved_thinking: Some(true),
            fps: None,
            network: NetworkConfig::default(),
            capabilities: CapabilitiesConfig::default(),
        }
    }
}

impl Config {
    pub fn load(working_dir: &Path) -> Result<Config> {
        let mut cfg = Config::default();
        // Merge ALL existing candidates, least-specific first so project files
        // override the global base (matches opencoder). This lets ~/.opencoder
        // provide the provider+key while a project opencoder.json overrides only
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
        warn_if_suspicious_model(&cfg.model);
        Ok(cfg)
    }
    pub fn model_id(&self) -> &str {
        self.model
            .split_once('/')
            .map(|(_, m)| m)
            .unwrap_or(&self.model)
    }
    pub fn provider_id(&self) -> &str {
        self.model
            .split_once('/')
            .map(|(p, _)| p)
            .unwrap_or("openai")
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
        self.api_key_for(self.provider_id())
    }

    /// Look up a named provider in the `providers` registry.
    pub fn provider_for(&self, name: &str) -> Option<&ProviderConfig> {
        self.providers.get(name)
    }

    /// Resolve the base_url for a provider name: `providers[name].base_url`
    /// if the name is registered, otherwise the legacy `provider.base_url`.
    pub fn base_url_for(&self, name: &str) -> String {
        match self.provider_for(name) {
            Some(p) => p.base_url.clone(),
            None => self.provider.base_url.clone(),
        }
    }

    /// Resolve the api_key for a provider name: `providers[name].api_key` →
    /// legacy `provider.api_key` → `OPENAI_API_KEY` env var.
    pub fn api_key_for(&self, name: &str) -> Result<String> {
        self.provider_for(name)
            .and_then(|p| p.api_key.clone())
            .or_else(|| self.provider.api_key.clone())
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| CoreError::Config("missing OPENAI_API_KEY".into()))
    }

    /// One-shot endpoint resolution for the current `model`'s provider prefix.
    /// Returns `(base_url, api_key)` ready for `ChatClient::new`.
    pub fn resolve_endpoint(&self) -> Result<(String, String)> {
        let name = self.provider_id();
        Ok((self.base_url_for(name), self.api_key_for(name)?))
    }

    /// Effective TUI frame rate (FPS), clamped to 1..=30. `None` -> 10.
    pub fn tui_fps(&self) -> u32 {
        self.fps.unwrap_or(10).clamp(1, 30)
    }

    /// Frame interval in milliseconds derived from [`tui_fps`](Self::tui_fps).
    pub fn tui_frame_ms(&self) -> u64 {
        1000 / self.tui_fps() as u64
    }

    /// Pick the file to persist config edits to. Rule (project-first, global
    /// fallback): the first existing candidate that already holds any of the
    /// editable keys; if none, create the project-local `./opencoder.json`.
    pub fn save_target(working_dir: &Path) -> PathBuf {
        let candidates = config_candidates(working_dir);
        // candidates are ordered project-first (index 0) → global-last, which
        // is exactly the priority we want for picking a save target.
        for p in &candidates {
            if p.exists() {
                if let Ok(raw) = std::fs::read_to_string(p) {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                        if has_editable_key(&v) {
                            return p.clone();
                        }
                    }
                }
            }
        }
        // Nothing editable on disk yet → create the project-local opencoder.json
        // at the working-dir root (more idiomatic than .opencoder/config.json).
        working_dir.join("opencoder.json")
    }

    /// Merge `patch` into the JSON at `save_target`, preserving unrelated keys
    /// and pretty-printing. Creates the file (and parent `.opencoder/` dir) if
    /// missing. Returns the path written.
    pub fn save(working_dir: &Path, patch: &serde_json::Value) -> Result<PathBuf> {
        let target = Self::save_target(working_dir);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let mut root: serde_json::Value = if target.exists() {
            std::fs::read_to_string(&target)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_else(|| serde_json::json!({}))
        } else {
            serde_json::json!({})
        };
        merge_json(&mut root, patch);
        let pretty = serde_json::to_string_pretty(&root)?;
        std::fs::write(&target, pretty)?;
        Ok(target)
    }
}

/// `true` if `root` (a parsed config file) carries any of the editable
/// top-level or nested keys the `/model` menu can write.
fn has_editable_key(root: &serde_json::Value) -> bool {
    let obj = match root.as_object() {
        Some(o) => o,
        None => return false,
    };
    if obj.contains_key("model")
        || obj.contains_key("small_model")
        || obj.contains_key("max_tokens")
        || obj.contains_key("reasoning_effort")
        || obj.contains_key("interleaved_thinking")
        || obj.contains_key("context_limit")
        || obj.contains_key("fps")
    {
        return true;
    }
    if obj
        .get("provider")
        .and_then(|v| v.as_object())
        .is_some_and(|p| p.contains_key("base_url") || p.contains_key("api_key"))
    {
        return true;
    }
    if obj
        .get("providers")
        .and_then(|v| v.as_object())
        .is_some_and(|p| !p.is_empty())
    {
        return true;
    }
    if obj
        .get("compaction")
        .and_then(|v| v.as_object())
        .is_some_and(|c| c.contains_key("context_threshold") || c.contains_key("auto"))
    {
        return true;
    }
    if obj
        .get("network")
        .and_then(|v| v.as_object())
        .is_some_and(|n| n.contains_key("proxy"))
    {
        return true;
    }
    if obj
        .get("capabilities")
        .and_then(|v| v.as_object())
        .is_some_and(|c| {
            c.contains_key("browser")
                || c.contains_key("computer_use")
                || c.contains_key("tools_subagent")
        })
    {
        return true;
    }
    false
}

/// Recursive JSON object merge: `patch` wins; nested objects are merged
/// key-by-key rather than replaced wholesale, so editing `compaction.context_threshold`
/// preserves a sibling `tail_turns`.
fn merge_json(dst: &mut serde_json::Value, patch: &serde_json::Value) {
    use serde_json::Value;
    match (dst, patch) {
        (Value::Object(d), Value::Object(p)) => {
            for (k, pv) in p {
                match (d.get_mut(k), pv) {
                    (Some(Value::Object(_)), Value::Object(_)) => {
                        if let Some(child) = d.get_mut(k) {
                            merge_json(child, pv);
                        }
                    }
                    (_, Value::Null) => {
                        d.remove(k);
                    }
                    _ => {
                        d.insert(k.clone(), pv.clone());
                    }
                }
            }
        }
        (d, p) => {
            *d = p.clone();
        }
    }
}

/// `true` when `s` looks like an environment-variable name (uppercase +
/// underscores/digits). Used by the `/model` menu to decide whether to wrap an
/// api-key value as `"{NAME}"` (preserving env-var indirection via
/// `resolve_env`) or store it verbatim.
pub fn looks_like_env_var(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty()
        && t.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && t.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

fn config_candidates(working_dir: &Path) -> Vec<PathBuf> {
    let mut v = vec![
        working_dir.join(".opencoder").join("config.json"),
        working_dir.join("opencoder.json"),
    ];
    if let Some(home) = dirs::home_dir() {
        // ~/.opencoder/ (this binary's own config home) — highest-priority global,
        // so `opencoder` runs directly from any directory with no project config.
        v.push(home.join(".opencoder").join("config.json"));
        v.push(home.join(".opencoder").join("opencoder.json"));
    }
    if let Some(cfg) = dirs::config_dir() {
        v.push(cfg.join("opencoder").join("config.json"));
    }
    v
}

fn apply_env(cfg: &mut Config) {
    if let Ok(b) = std::env::var("OPENAI_BASE_URL") {
        if !b.is_empty() {
            cfg.provider.base_url = b.trim_end_matches('/').to_string();
        }
    }
    if let Ok(m) = std::env::var("OPENCODER_MODEL") {
        if !m.is_empty() {
            cfg.model = m;
        }
    }
    if let Ok(m) = std::env::var("OPENCODER_SMALL_MODEL") {
        if !m.is_empty() {
            cfg.small_model = Some(m);
        }
    }
    if let Ok(v) = std::env::var("OPENCODER_CONTEXT_LIMIT") {
        if let Ok(n) = v.parse::<u64>() {
            cfg.context_limit = Some(n);
        }
    }
    if let Ok(raw) = std::env::var("OPENCODER_CACHE_SALT") {
        match raw.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => cfg.cache_salt = Some(true),
            "false" | "0" | "no" => cfg.cache_salt = Some(false),
            _ => {}
        }
    }
    // Proxy overlay: explicit OPENCODER_PROXY wins, then ALL_PROXY. Only set
    // when the user has not already configured `network.proxy` directly.
    if cfg.network.proxy.is_none() {
        for var in ["OPENCODER_PROXY", "ALL_PROXY"] {
            if let Ok(v) = std::env::var(var) {
                let t = v.trim();
                if !t.is_empty() {
                    cfg.network.proxy = Some(t.to_string());
                    break;
                }
            }
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
        if let Some(re) = obj.get("reasoning_effort").and_then(|v| v.as_str()) {
            let trimmed = re.trim();
            if trimmed.is_empty() {
                cfg.reasoning_effort = None;
            } else {
                cfg.reasoning_effort = Some(trimmed.to_string());
            }
        }
        if let Some(it) = obj.get("interleaved_thinking").and_then(|v| v.as_bool()) {
            cfg.interleaved_thinking = Some(it);
        }
        if let Some(v) = obj.get("cache_salt").and_then(|v| v.as_bool()) {
            cfg.cache_salt = Some(v);
        }
        if let Some(fps) = obj.get("fps").and_then(|v| v.as_u64()) {
            cfg.fps = Some(fps.clamp(1, 30) as u32);
        }
        if let Some(p) = obj.get("provider").and_then(|v| v.as_object()) {
            if let Some(b) = p.get("base_url").and_then(|v| v.as_str()) {
                cfg.provider.base_url = b.to_string();
            }
            if let Some(k) = p.get("api_key").and_then(|v| v.as_str()) {
                cfg.provider.api_key = Some(resolve_env(k));
            }
        }
        if let Some(providers) = obj.get("providers").and_then(|v| v.as_object()) {
            for (name, pv) in providers {
                if let Some(pcfg) = pv.as_object() {
                    let entry = cfg
                        .providers
                        .entry(name.clone())
                        .or_default();
                    if let Some(b) = pcfg.get("base_url").and_then(|v| v.as_str()) {
                        entry.base_url = b.to_string();
                    }
                    if let Some(k) = pcfg.get("api_key").and_then(|v| v.as_str()) {
                        entry.api_key = Some(resolve_env(k));
                    }
                    if let Some(m) = pcfg.get("model").and_then(|v| v.as_str()) {
                        entry.model = Some(m.to_string());
                    }
                }
            }
        }
        if let Some(c) = obj.get("compaction").and_then(|v| v.as_object()) {
            if let Some(v) = c.get("auto").and_then(|v| v.as_bool()) {
                cfg.compaction.auto = v;
            }
            if let Some(v) = c.get("context_threshold").and_then(|v| v.as_u64()) {
                cfg.compaction.context_threshold = v;
            }
            if let Some(v) = c.get("tail_turns").and_then(|v| v.as_u64()) {
                cfg.compaction.tail_turns = v as u32;
            }
            if let Some(v) = c.get("reserved").and_then(|v| v.as_u64()) {
                cfg.compaction.reserved = v;
            }
            if let Some(v) = c.get("buffer").and_then(|v| v.as_u64()) {
                cfg.compaction.buffer = Some(v);
            }
        }
        if let Some(a) = obj.get("agent").and_then(|v| v.as_object()) {
            if let Some(d) = a.get("default").and_then(|v| v.as_str()) {
                cfg.agent.default = d.to_string();
            }
        }
        if let Some(n) = obj.get("network").and_then(|v| v.as_object()) {
            if let Some(p) = n.get("proxy").and_then(|v| v.as_str()) {
                let t = p.trim();
                cfg.network.proxy = if t.is_empty() { None } else { Some(t.to_string()) };
            }
        }
        if let Some(c) = obj.get("capabilities").and_then(|v| v.as_object()) {
            if let Some(b) = c.get("browser").and_then(|v| v.as_bool()) {
                cfg.capabilities.browser = b;
            }
            if let Some(b) = c.get("computer_use").and_then(|v| v.as_bool()) {
                cfg.capabilities.computer_use = b;
            }
            if let Some(b) = c.get("tools_subagent").and_then(|v| v.as_bool()) {
                cfg.capabilities.tools_subagent = b;
            }
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

#[cfg(test)]
mod tests {
    use super::is_suspicious_model;

    #[test]
    fn empty_model_is_not_suspicious() {
        assert!(!is_suspicious_model(""));
    }

    #[test]
    fn well_formed_scoped_model_is_not_suspicious() {
        assert!(!is_suspicious_model("openai/gpt-4o"));
        assert!(!is_suspicious_model("anthropic/claude-3.5-sonnet"));
    }

    #[test]
    fn boundary_two_char_sides_are_not_suspicious() {
        // provider.len() == 2 && mid.len() == 2 is the minimum valid scoped model
        assert!(!is_suspicious_model("ab/cd"));
    }

    #[test]
    fn unscoped_short_model_is_suspicious() {
        assert!(is_suspicious_model("x")); // len < 3, no slash
    }

    #[test]
    fn unscoped_three_char_model_is_not_suspicious() {
        // len == 3 is the minimum valid unscoped model (boundary)
        assert!(!is_suspicious_model("abc"));
    }

    #[test]
    fn short_provider_side_is_suspicious() {
        assert!(is_suspicious_model("a/bc")); // provider.len() < 2
    }

    #[test]
    fn short_model_side_is_suspicious() {
        assert!(is_suspicious_model("ab/c")); // mid.len() < 2
    }
}

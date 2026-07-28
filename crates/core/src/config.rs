use crate::error::{CoreError, Result};
use crate::tool_guard_config::ToolGuardConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

mod autopilot;
mod merge;

pub use autopilot::AutoPilotConfig;

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
    /// Per-message assistant-output streamlining (deterministic, meaning-
    /// preserving). See [`OutputStreamlineConfig`].
    #[serde(default)]
    pub output_streamline: OutputStreamlineConfig,
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
    #[serde(
        default = "default_cache_salt",
        skip_serializing_if = "Option::is_none"
    )]
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
    /// Tool-failure guard: consecutive-failure threshold and exponential
    /// backoff. Defaults: 5 consecutive failures → abort; 200 ms → 2000 ms
    /// exponential backoff.
    #[serde(default)]
    pub tool_guard: ToolGuardConfig,
    /// Max idle duration (no LLM stream events received) before a streaming
    /// call is considered stalled and aborted (seconds). Defaults to 600.
    /// Independent of the HTTP read_timeout — catches stalls where the upstream
    /// keeps the connection alive with SSE comment frames but never delivers
    /// actual content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_idle_timeout_secs: Option<u64>,
    /// Max wall-clock duration for a `task` subagent before it is aborted
    /// (seconds). Defaults to 1800 (30 min). Prevents indefinite subagent hangs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_timeout_secs: Option<u64>,
    /// Autopilot loop (PLAN -> ACT -> VERIFY). Off by default.
    #[serde(default)]
    pub autopilot: AutoPilotConfig,
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
            "config `model` looks malformed (expected `provider/model`, e.g. `openai/gpt-4o`); fix the `model` field in your config file or set the matching env var if this is a stale value"
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
    /// Extra HTTP headers attached to every request to this provider. A header
    /// `value` may be a literal string or a `{VAR}` reference resolved from the
    /// environment at endpoint-resolution time (same convention as `api_key`).
    #[serde(default)]
    pub headers: Vec<HttpHeader>,
}

/// A custom HTTP header applied to provider requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

/// Resolved provider endpoint: everything `ChatClient::new` needs to talk to
/// the model's provider. `headers` are env-resolved name/value pairs; a custom
/// header sharing a built-in name (e.g. `authorization`, `content-type`)
/// overrides the built-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub base_url: String,
    pub api_key: String,
    pub headers: Vec<(String, String)>,
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
/// Per-message assistant-output streamlining. Deterministic, meaning-preserving
/// normalization applied to completed assistant text *after* it has been
/// streamed to the UI (so live display fidelity is untouched) and *before* it
/// is persisted / re-sent as context — shaving **input** token overhead on
/// every later turn. Fenced code blocks are passed through verbatim; only
/// prose whitespace/structure is touched, so it is a no-op on already-clean
/// text. Configured via the `output_streamline` field of [`Config`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputStreamlineConfig {
    /// Master switch. On by default — every rule is a no-op on clean text.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Strip trailing whitespace from each prose line.
    #[serde(default = "default_true")]
    pub trim_trailing: bool,
    /// Collapse runs of 2+ blank prose lines into a single blank line.
    #[serde(default = "default_true")]
    pub collapse_blank_lines: bool,
    /// Trim leading/trailing blank lines from the whole message.
    #[serde(default = "default_true")]
    pub trim_outer: bool,
    /// Collapse interior space/tab runs in prose to a single space (leading
    /// indentation is preserved). Off by default: opt-in "aggressive" mode.
    #[serde(default)]
    pub collapse_inline_ws: bool,
}

impl Default for OutputStreamlineConfig {
    fn default() -> Self {
        OutputStreamlineConfig {
            enabled: true,
            trim_trailing: true,
            collapse_blank_lines: true,
            trim_outer: true,
            collapse_inline_ws: false,
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
            output_streamline: OutputStreamlineConfig::default(),
            context_limit: None,
            max_tokens: None,
            reasoning_effort: None,
            cache_salt: default_cache_salt(),
            interleaved_thinking: Some(true),
            fps: None,
            network: NetworkConfig::default(),
            capabilities: CapabilitiesConfig::default(),
            tool_guard: ToolGuardConfig::default(),
            stream_idle_timeout_secs: None,
            task_timeout_secs: None,
            autopilot: AutoPilotConfig::default(),
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
                merge::merge_into(&mut cfg, parsed);
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
    /// Effective stream idle timeout for LLM streaming calls. When no events
    /// are received within this duration, the call is aborted to prevent
    /// indefinite hangs from stalled connections.
    pub fn stream_idle_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.stream_idle_timeout_secs.unwrap_or(600))
    }
    /// Effective max wall-clock duration for a single `task` subagent.
    pub fn task_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.task_timeout_secs.unwrap_or(1800))
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
    /// Returns an [`Endpoint`] ready for `ChatClient::new`. Header `value`s are
    /// env-resolved (a `{VAR}` reference expands to the env var; anything else
    /// is used literally). When the provider is not in the `providers` map, the
    /// legacy top-level `provider` field supplies base_url/api_key/headers.
    pub fn resolve_endpoint(&self) -> Result<Endpoint> {
        let name = self.provider_id();
        let headers_src = match self.provider_for(name) {
            Some(p) => &p.headers,
            None => &self.provider.headers,
        };
        let headers: Vec<(String, String)> = headers_src
            .iter()
            .map(|h| (h.name.clone(), resolve_env(&h.value)))
            .collect();
        Ok(Endpoint {
            base_url: self.base_url_for(name),
            api_key: self.api_key_for(name)?,
            headers,
        })
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
                        if merge::has_editable_key(&v) {
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
        merge::merge_json(&mut root, patch);
        // Guard: refuse to persist a malformed `model` (e.g. `m/g`). Such a value
        // would make every downstream request fail silently (`model_id()` resolves
        // to a single char). Surface the error so the caller shows it to the user
        // instead of corrupting the config file. See is_suspicious_model for the
        // predicate.
        if let Some(model) = root.get("model").and_then(|v| v.as_str()) {
            if is_suspicious_model(model) {
                return Err(CoreError::Config(format!(
                    "refusing to write malformed `model` value `{model}`: expected \
                     `provider/model` with each side at least 2 chars (e.g. \
                     `openai/gpt-4o`); edit the `model` field in your config file"
                )));
            }
        }
        let pretty = serde_json::to_string_pretty(&root)?;
        std::fs::write(&target, pretty)?;
        Ok(target)
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

pub(super) fn resolve_env(raw: &str) -> String {
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
    use super::{is_suspicious_model, Config};

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

    #[test]
    fn stream_idle_timeout_defaults_to_600s() {
        assert_eq!(
            Config::default().stream_idle_timeout(),
            std::time::Duration::from_secs(600)
        );
    }

    #[test]
    fn stream_idle_timeout_is_configurable() {
        let c = Config {
            stream_idle_timeout_secs: Some(60),
            ..Default::default()
        };
        assert_eq!(c.stream_idle_timeout(), std::time::Duration::from_secs(60));
    }

    #[test]
    fn task_timeout_defaults_to_1800s() {
        assert_eq!(
            Config::default().task_timeout(),
            std::time::Duration::from_secs(1800)
        );
    }

    #[test]
    fn task_timeout_is_configurable() {
        let c = Config {
            task_timeout_secs: Some(300),
            ..Default::default()
        };
        assert_eq!(c.task_timeout(), std::time::Duration::from_secs(300));
    }
}

use crate::error::{CoreError, Result};
use crate::tool_guard_config::ToolGuardConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

mod autopilot;
mod cli;
mod domain;
mod env;
pub mod envs;
mod keymap;
mod mcp;
pub(crate) mod mcp_guard;
mod merge;
pub mod redact;
mod skill;

pub use mcp_guard::{mcp_name_collision, mcp_name_conflict_in_patch};

pub use autopilot::{ApMode, AutoPilotConfig};
pub use cli::{CliConfig, CliToolConfig, InjectionTarget};
pub use env::{looks_like_env_var, scoped_config_home, ScopedConfigHome};
pub use envs::{
    active_env, create_env, delete_env, env_dir, envs_home, list_envs, recapture_env,
    set_active_env, validate_env_name,
};
pub use keymap::KeymapConfig;
pub use keymap::KEYMAP_INFO;
pub use mcp::McpServerConfig;
pub use skill::SkillConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub provider: ProviderConfig,
    /// Named OpenAI-compatible providers. Each entry is `{base_url, api_key?, model?}`.
    /// The active provider is selected by the `provider/` prefix of `model`.
    /// Empty by default; populate via config file. No built-in presets.
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    /// Named MCP servers. Only entries with `enabled == true` are surfaced.
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,
    /// Named CLI usage contracts. Enabled entries are injected into the system prompt.
    #[serde(default)]
    pub cli: HashMap<String, CliConfig>,
    /// Named skill default-injection toggles. Only `enabled == true` entries are
    /// surfaced (as names in the context-tail skill catalog reminder).
    #[serde(default)]
    pub skills: HashMap<String, SkillConfig>,
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
    /// field on the chat request body. Accepted values: `low|medium|high|xhigh|max`.
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
    /// Outbound proxy for LLM traffic. Accepts `socks5://`,
    /// `socks5h://`, `http://`, `https://`. The effective value also honors
    /// `OPENCODER_PROXY` / `ALL_PROXY` env vars (see `net::effective_proxy`).
    #[serde(default)]
    pub network: NetworkConfig,
    /// Tool-failure guard: consecutive-failure threshold and exponential
    /// backoff. Defaults: 20 consecutive failures → abort; 200 ms → 2000 ms
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
    /// Per-step idle timeout for a `task` subagent (seconds). Defaults to 1800
    /// (30 min). The deadline resets on every forward-progress signal the child
    /// produces (tool call start/end, LLM text/reasoning deltas), so a
    /// long-running but active subagent is never killed — the timeout fires only
    /// when a single step stalls with no activity for this long. Formerly a
    /// single wall-clock cap; behaviour changed to idle-timeout semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_timeout_secs: Option<u64>,
    /// Max wall-clock duration for replaying a single interrupted subagent
    /// during session recovery (seconds). Defaults to 300 (5 min). Shorter than
    /// `task_timeout_secs` because recovery should not block the user for 30 min.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay_timeout_secs: Option<u64>,
    /// Grace window (seconds) given to a subagent to finish its cleanup after an
    /// interrupt (hard cancel / turn cancel / timeout) before the runner forces
    /// the task into the Cancelled state. Defaults to 15.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_drain_secs: Option<u64>,
    /// Autopilot loop (PLAN -> ACT -> VERIFY). Off by default.
    #[serde(default)]
    pub autopilot: AutoPilotConfig,
    /// When true, bare `opencode` wraps the TUI in a tmux session (so it
    /// survives SSH disconnect). Off by default; requires tmux installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_tmux_session: Option<bool>,
    /// User-configurable keyboard shortcuts (see [`KEYMAP_INFO`]).
    #[serde(default)]
    pub keymap: KeymapConfig,
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
/// Pure predicate: is the `model` string malformed (empty, too short on
/// either side of the `/`, or too short unscoped)? `pub` for cli/web checks.
pub fn is_suspicious_model(model: &str) -> bool {
    if model.is_empty() {
        return true;
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

/// Serde default for [`AgentDefaults::default`], kept in sync with the
/// `Default` impl so deserializing `{}` yields `"act"` rather than `""`.
/// (Returns `String` to match the field type for `#[serde(default = ...)]`.)
fn default_agent_name() -> String {
    "act".to_string()
}

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
    #[serde(default = "default_agent_name")]
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

/// Networking options for outbound LLM traffic.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkConfig {
    /// Proxy URL (`socks5://`, `socks5h://`, `http://`, `https://`). `None`
    /// means a direct connection (subject to env-var fallback at use time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            provider: ProviderConfig {
                base_url: default_base_url(),
                ..Default::default()
            },
            providers: HashMap::new(),
            mcp_servers: HashMap::new(),
            cli: HashMap::new(),
            skills: HashMap::new(),
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
            tool_guard: ToolGuardConfig::default(),
            stream_idle_timeout_secs: None,
            task_timeout_secs: None,
            replay_timeout_secs: None,
            subagent_drain_secs: None,
            autopilot: AutoPilotConfig::default(),
            enable_tmux_session: None,
            keymap: KeymapConfig::default(),
        }
    }
}

impl Config {
    /// Canonical user-global config path: `~/.opencoder/config.json`.
    /// Test callers using [`scoped_config_home`] receive the isolated path.
    pub fn global_config_path() -> Result<PathBuf> {
        env::primary_global_config_path().ok_or_else(|| {
            CoreError::Config("cannot resolve home directory for ~/.opencoder/config.json".into())
        })
    }

    /// Ensure the canonical global config exists without overwriting it.
    /// Returns `(path, created)`; a newly-created file contains an empty JSON
    /// object so a cancelled first-run wizard can safely resume next launch.
    pub fn ensure_global_config() -> Result<(PathBuf, bool)> {
        use std::io::Write;

        let path = Self::global_config_path()?;
        if path.exists() {
            return Ok((path, false));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut file) => {
                file.write_all(b"{}\n")?;
                Ok((path, true))
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok((path, false)),
            Err(e) => Err(e.into()),
        }
    }

    /// Return a cloned config with `patch` applied using the same merge rules
    /// as disk loading/saving. The source config is never mutated. Domain
    /// keys (`mcp_servers` / `cli` / `skills` / `autopilot`) still apply
    /// here — they are routed through the same per-entry domain merge, so
    /// building configs
    /// from JSON patches keeps working even though `config.json` itself no
    /// longer carries them.
    pub fn merged_with(&self, patch: &serde_json::Value) -> Config {
        let mut merged = self.clone();
        let (remainder, domains) = domain::split_patch(patch);
        merge::merge_into(&mut merged, remainder);
        for (key, value) in &domains {
            domain::apply_domain(&mut merged, key, value);
        }
        merged
    }

    pub fn load(working_dir: &Path) -> Result<Config> {
        let mut cfg = Config::default();
        // Merge ALL existing candidates, least-specific first so project files
        // override the global base (matches opencoder). This lets ~/.opencoder
        // provide the provider+key while a project opencoder.json overrides only
        // the model — `opencoder` then runs directly from any directory.
        let mut candidates = env::config_candidates(working_dir);
        candidates.reverse(); // global first, project last (wins)
        for p in candidates {
            if p.exists() {
                let raw = std::fs::read_to_string(&p)?;
                let parsed: serde_json::Value = serde_json::from_str(&raw)?;
                if !parsed.is_object() {
                    // A valid-JSON-but-not-object file (e.g. `[1,2]` or
                    // `"foo"`) falls through `merge_into` silently. Warn so the
                    // misconfiguration is visible instead of dropped.
                    let kind = match &parsed {
                        serde_json::Value::Null => "null",
                        serde_json::Value::Bool(_) => "bool",
                        serde_json::Value::Number(_) => "number",
                        serde_json::Value::String(_) => "string",
                        serde_json::Value::Array(_) => "array",
                        serde_json::Value::Object(_) => "object",
                    };
                    tracing::warn!(
                        "config file {} is valid JSON but not an object (got \
                         {}); ignoring",
                        p.display(),
                        kind
                    );
                }
                merge::merge_into(&mut cfg, parsed);
            }
        }
        // Domain files (mcp.json / cli.json / skills.json / ap.json):
        // `mcp_servers` / `cli` / `skills` / `autopilot` are hard-cut from
        // config.json and load from exactly one file — the project one when
        // it exists, else the global one (project shadows global entirely;
        // no per-key merge across files).
        for (key, _) in domain::DOMAIN_FILES {
            if let Some(v) = domain::read_effective(working_dir, key) {
                domain::apply_domain(&mut cfg, key, &v);
            }
        }
        env::apply_env(&mut cfg);
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
    /// Effective per-step idle timeout for a single `task` subagent. The
    /// deadline resets on every child activity signal, so this bounds how long a
    /// single stalled step (no events) may run — not total subagent runtime.
    pub fn task_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.task_timeout_secs.unwrap_or(1800))
    }
    /// Effective max wall-clock duration for replaying a single interrupted
    /// subagent during session recovery. Caps how long `resume_and_replay` /
    /// `replay_cancelled_tasks` will block the user while re-running a child.
    pub fn replay_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.replay_timeout_secs.unwrap_or(300))
    }
    /// Effective grace window for a subagent to drain after an interrupt.
    pub fn subagent_drain(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.subagent_drain_secs.unwrap_or(15))
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

    /// Returns enabled MCP servers sorted by name: `(name, config)` pairs.
    pub fn enabled_mcp_servers(&self) -> Vec<(String, &McpServerConfig)> {
        let mut out: Vec<(String, &McpServerConfig)> = self
            .mcp_servers
            .iter()
            .filter(|(_, c)| c.enabled)
            .map(|(n, c)| (n.clone(), c))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Returns enabled MCP servers applicable to the agent session `name`
    /// running in `mode` (primary agents share the `parent` flag; subagents
    /// are matched by name — see [`InjectionTarget::allows_agent`]).
    pub fn enabled_mcp_servers_for(
        &self,
        name: &str,
        mode: crate::AgentMode,
    ) -> Vec<(String, &McpServerConfig)> {
        self.enabled_mcp_servers()
            .into_iter()
            .filter(|(_, cfg)| cfg.inject_to.allows_agent(name, mode))
            .collect()
    }

    /// Returns non-empty enabled CLI registrations sorted by name.
    pub fn enabled_cli(&self) -> Vec<(String, &CliConfig)> {
        let mut out: Vec<_> = self
            .cli
            .iter()
            .filter(|(_, cfg)| cfg.enabled && !cfg.content.trim().is_empty())
            .map(|(name, cfg)| (name.clone(), cfg))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Returns enabled CLI registrations applicable to the agent session
    /// `name` running in `mode` (see [`InjectionTarget::allows_agent`]).
    pub fn enabled_cli_for(&self, name: &str, mode: crate::AgentMode) -> Vec<(String, &CliConfig)> {
        self.enabled_cli()
            .into_iter()
            .filter(|(_, cfg)| cfg.inject_to.allows_agent(name, mode))
            .collect()
    }

    /// Returns names of skills enabled for default injection, sorted by name.
    pub fn enabled_skill_names(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .skills
            .iter()
            .filter(|(_, c)| c.enabled)
            .map(|(n, _)| n.clone())
            .collect();
        out.sort();
        out
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
    /// legacy `provider.api_key` → `OPENAI_API_KEY` env var (skipped when a
    /// test isolation override is active on this thread).
    pub fn api_key_for(&self, name: &str) -> Result<String> {
        self.provider_for(name)
            .and_then(|p| p.api_key.clone())
            .or_else(|| self.provider.api_key.clone())
            .or_else(|| env::env_get("OPENAI_API_KEY"))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| CoreError::Config(format!("missing API key for provider `{name}`: set \
                `providers.{name}.api_key`, top-level `provider.api_key`, or the `OPENAI_API_KEY` env var")))
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
            .map(|h| (h.name.clone(), env::resolve_env(&h.value)))
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
    /// While an env is active the env's config.json is the terminal target:
    /// global/XDG candidates are skipped so `/model`-style edits land in the
    /// env and the base files stay pristine for deactivation.
    pub fn save_target(working_dir: &Path) -> PathBuf {
        let active = envs::active_env();
        let mut candidates = env::config_candidates_with(working_dir, active.as_deref());
        if active.is_some() {
            // candidate layout: 2 project entries + 1 env entry; drop the
            // global/XDG tail (active_env() validated the env dir, so the
            // env candidate is always present here).
            candidates.truncate(3);
        }
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
        // With an env active, create the env's config.json instead so the edit
        // stays env-scoped (deactivation restores the base config verbatim).
        match active.as_deref().and_then(envs::env_dir) {
            Some(dir) => dir.join("config.json"),
            None => working_dir.join("opencoder.json"),
        }
    }

    /// Split-routing save (分流): top-level domain keys (`mcp_servers` /
    /// `cli` / `skills` / `autopilot`) are written to their dedicated domain
    /// files (`mcp.json` / `cli.json` / `skills.json` / `ap.json`); the
    /// remainder follows the
    /// normal [`save_target`](Self::save_target) + [`save_to`] config.json
    /// flow.
    ///
    /// Return-path semantics: a non-empty config remainder writes config.json
    /// and returns its path (domain writes still happen); a patch containing
    /// only domain keys returns the last domain write target and never
    /// creates a config.json; an empty patch with no domain keys keeps the
    /// legacy config.json-only behavior.
    pub fn save(working_dir: &Path, patch: &serde_json::Value) -> Result<PathBuf> {
        let (remainder, domains) = domain::split_patch(patch);
        let mut last_domain: Option<PathBuf> = None;
        for (key, value) in &domains {
            let target = domain::save_domain(working_dir, key, value)
                .map_err(|e| CoreError::Config(format!("save domain file for `{key}`: {e}")))?;
            last_domain = Some(target);
        }
        if remainder.as_object().is_some_and(|o| !o.is_empty()) {
            let target = Self::save_target(working_dir);
            return Self::save_to(&target, &remainder);
        }
        if let Some(target) = last_domain {
            return Ok(target);
        }
        let target = Self::save_target(working_dir);
        Self::save_to(&target, patch)
    }

    /// Merge a patch into the canonical global config, regardless of project
    /// config precedence. Used by first-run onboarding; normal `/model` saves
    /// continue to use [`save`](Self::save).
    pub fn save_global(patch: &serde_json::Value) -> Result<PathBuf> {
        let _ = Self::ensure_global_config()?;
        let target = Self::global_config_path()?;
        Self::save_to(&target, patch)
    }

    fn save_to(target: &Path, patch: &serde_json::Value) -> Result<PathBuf> {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut root: serde_json::Value = if target.exists() {
            let raw = std::fs::read_to_string(target)
                .map_err(|e| CoreError::Config(format!("read config {}: {e}", target.display())))?;
            match serde_json::from_str::<serde_json::Value>(&raw) {
                Ok(v) => v,
                Err(e) => {
                    // Don't silently destroy a corrupt file — surface the
                    // error. An empty/whitespace-only file is treated as an
                    // empty object (matches a freshly-created config).
                    if raw.trim().is_empty() {
                        serde_json::json!({})
                    } else {
                        return Err(CoreError::Config(format!(
                            "config file {} is corrupt: {e}; refusing to overwrite",
                            target.display()
                        )));
                    }
                }
            }
        } else {
            serde_json::json!({})
        };
        merge::merge_json(&mut root, patch);
        // MCP name-collision guard (bug #14): two `mcp_servers` names that
        // normalize to the same tool prefix would shadow each other's tools
        // at registration. Defensive for paths that still carry the key
        // through config.json (`save_global` / the empty-patch fallback) —
        // normal saves route `mcp_servers` to mcp.json, guarded in
        // `domain::save_domain`. Runs before any write: nothing half-done.
        if let Some(servers) = root.get("mcp_servers").and_then(|v| v.as_object()) {
            if let Some((offending, existing)) = mcp_guard::mcp_name_collision(servers) {
                return Err(CoreError::Config(mcp_guard::conflict_message(
                    &offending, &existing,
                )));
            }
        }
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
        std::fs::write(target, pretty)?;
        Ok(target.to_path_buf())
    }
}

#[cfg(test)]
mod tests;

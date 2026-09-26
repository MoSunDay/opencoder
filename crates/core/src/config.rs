use crate::error::{CoreError, Result};
use crate::tool_guard_config::ToolGuardConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Persist a config save to `target` (body already pretty-printed).
pub(crate) fn write_config_save(target: &Path, body: &str) -> std::io::Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(target, body)
}

mod agent;
mod autopilot;
mod cli;
mod compaction;
mod dag;
mod domain;
pub(crate) mod env;
mod keymap;
mod mcp;
pub(crate) mod mcp_guard;
mod merge;
mod model_guard;
mod persistence;
mod provider;
pub mod redact;
mod schedule;
mod skill;
mod storage;

pub use mcp_guard::{mcp_name_collision, mcp_name_conflict_in_patch};

pub use agent::{AgentDefaults, AgentNfsConfig, ToolsScope};
pub use autopilot::{ApMode, AutoPilotConfig};
pub use cli::{CliConfig, InjectionTarget};
pub use compaction::{CompactionConfig, OutputStreamlineConfig};
pub use dag::{AgentSandbox, DagConfig, DagDeviceConfig, DagOpConfig};
pub use env::{looks_like_env_var, scoped_config_home, ScopedConfigHome};
pub use keymap::KeymapConfig;
pub use keymap::KEYMAP_INFO;
pub use mcp::McpServerConfig;
pub use model_guard::is_suspicious_model;
pub use provider::{Endpoint, HttpHeader, ProviderConfig};
pub use schedule::{
    load_schedules, schedules_path, validate_id as validate_schedule_id, ScheduleJob, ScheduleKind,
    ScheduleOverlap, SchedulesConfig,
};
pub use skill::SkillConfig;
pub use storage::{StorageBackend, StorageConfig};

use model_guard::warn_if_suspicious_model;

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
    /// Run repository memory maintenance in an isolated context after each task.
    #[serde(default)]
    pub local_memory: bool,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub small_model: Option<String>,
    /// Model id for `/embeddings` calls; `None` → [`Config::embedding_model_id`]'s default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    /// Provider name whose endpoint serves `/embeddings` calls; `None` →
    /// embeddings ride the primary provider (see [`Config::resolve_embedding_endpoint`]).
    /// Lets chat stay on a remote provider while embeddings run on a local
    /// model server (e.g. ollama with bge-m3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_provider: Option<String>,
    #[serde(default)]
    pub agent: AgentDefaults,
    /// DAG wasm-module pool + NFS export knobs.
    #[serde(default)]
    pub dag: DagConfig,
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
    /// Cron-scheduled control-plane jobs. Hard-cut into the `schedules.json`
    /// domain file (never carried in config.json); see [`config::domain`].
    #[serde(default, skip_serializing_if = "SchedulesConfig::is_empty")]
    pub schedules: SchedulesConfig,
    /// When true, bare `opencoder` wraps the TUI in a tmux session (so it
    /// survives SSH disconnect). Off by default; requires tmux installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_tmux_session: Option<bool>,
    /// User-configurable keyboard shortcuts (see [`KEYMAP_INFO`]).
    #[serde(default)]
    pub keymap: KeymapConfig,
    /// Project-data storage backend selection (libsql default; optional
    /// mysql/starrocks for project tables only). DSNs may use `{VAR}` refs.
    #[serde(default)]
    pub storage: StorageConfig,
    /// opencoder-team workspace root: per-topic scratch/checkpoint area
    /// shared by the multi-node topic fan-out. Default `<data_root>/team`.
    #[serde(default = "default_team_root")]
    pub team_root: PathBuf,
    /// Max outer turns per team topic run. Default 8.
    #[serde(default = "default_team_max_turns")]
    pub team_max_turns: usize,
    /// Max inner (sub) turns per outer turn of a team topic run. Default 3.
    #[serde(default = "default_team_max_sub_turns")]
    pub team_max_sub_turns: usize,
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

fn default_model() -> String {
    "openai/gpt-4o-mini".to_string()
}

// `<data_root>/team` sits beside, not inside, any single workdir's data dir.
fn default_team_root() -> PathBuf {
    crate::data_dir::data_root().join("team")
}

fn default_team_max_turns() -> usize {
    8
}

fn default_team_max_sub_turns() -> usize {
    3
}

/// The persisted team layout uses three-digit turn directories. Both runtime
/// budgets are counts, so zero is invalid and 999 is the largest value that
/// can be represented without reaching an illegal directory name.
pub const TEAM_TURN_BUDGET_MAX: usize = 999;

/// Validate the two team runtime budgets before any team state is created.
/// Pure so config loading and the runtime boundary share one contract.
pub fn validate_team_turn_budgets(max_turns: usize, max_sub_turns: usize) -> Result<()> {
    fn validate(name: &str, value: usize) -> Result<()> {
        if (1..=TEAM_TURN_BUDGET_MAX).contains(&value) {
            Ok(())
        } else {
            Err(CoreError::Config(format!(
                "{name} must be between 1 and {TEAM_TURN_BUDGET_MAX}, got {value}"
            )))
        }
    }

    validate("team_max_turns", max_turns)?;
    validate("team_max_sub_turns", max_sub_turns)
}

/// Default context window assumed when neither config nor a model registry
/// supplies one. Large enough that the `context_threshold` is the binding
/// constraint by default, but lets `reserved` take effect once set.
pub const DEFAULT_CONTEXT_LIMIT: u64 = 128_000;

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
                base_url: provider::default_base_url(),
                ..Default::default()
            },
            providers: HashMap::new(),
            mcp_servers: HashMap::new(),
            cli: HashMap::new(),
            skills: HashMap::new(),
            local_memory: false,
            model: default_model(),
            small_model: None,
            embedding_model: None,
            embedding_provider: None,
            agent: AgentDefaults::default(),
            dag: DagConfig::default(),
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
            schedules: SchedulesConfig::default(),
            enable_tmux_session: None,
            keymap: KeymapConfig::default(),
            storage: StorageConfig::default(),
            team_root: default_team_root(),
            team_max_turns: default_team_max_turns(),
            team_max_sub_turns: default_team_max_sub_turns(),
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
        Self::load_with_home(working_dir, None)
    }

    /// [`Config::load`] with the GLOBAL config home redirected to `home`:
    /// the `~/.opencoder` candidates and the global domain files
    /// (`mcp.json` / `cli.json` / ...) resolve inside `home`, while the
    /// project candidates still resolve against `working_dir`. Env overlays
    /// (`apply_env`) keep applying — isolation is about files, not env.
    ///
    /// Operator execution isolation uses this so a session's config
    /// reloads reproduce the execution's frozen snapshot instead of the
    /// node daemon user's live `~/.opencoder`.
    pub fn load_with_home(working_dir: &Path, home: Option<&Path>) -> Result<Config> {
        Self::load_inner(working_dir, home, true)
    }

    /// [`load_with_home`] with environment overlays skipped. This is the
    /// versioned operator execution contract: the snapshot is the final
    /// authority ("snapshot is final"), so `OPENCODER_MODEL` /
    /// `OPENAI_BASE_URL` / `OPENAI_API_KEY` and friends only participate ONCE
    /// — at creation time, when the snapshot is written. A later change to
    /// the node daemon's environment can no longer shift a running or
    /// resumed execution.
    pub fn load_with_home_frozen(working_dir: &Path, home: Option<&Path>) -> Result<Config> {
        Self::load_inner(working_dir, home, false)
    }

    /// Operator-plane configuration source: the ONLY file candidates are
    /// `dir/config.json` + `dir/opencoder.json` and the domain files
    /// `dir/<mcp|cli|skills|ap|schedules>.json`. Project candidates
    /// (`<workdir>/opencoder.json`, `<workdir>/.opencoder/*`), the interactive
    /// user's real `~/.opencoder`, XDG dirs and env overlays are all
    /// deliberately out of scope: the node operator plane reads exclusively
    /// its own directory, so TUI/CLI config writes can never reach an
    /// operator execution.
    pub fn load_operator(dir: &Path) -> Result<Config> {
        let mut cfg = Config::default();
        for p in [dir.join("config.json"), dir.join("opencoder.json")] {
            if p.exists() {
                let parsed: serde_json::Value =
                    serde_json::from_str(&std::fs::read_to_string(&p)?)?;
                crate::provider::validate_protocol_patch(&parsed)?;
                if parsed.is_object() {
                    merge::merge_into(&mut cfg, parsed);
                }
            }
        }
        for (key, file) in domain::DOMAIN_FILES {
            let path = dir.join(file);
            if path.is_file() {
                if let Some(v) = domain::read_effective_from(&path) {
                    domain::apply_domain(&mut cfg, key, &v);
                }
            }
        }
        Self::finalize_operator(cfg)
    }

    /// Shared tail of `load_with_home` / `load_operator`: scope overrides
    /// from the harness, no env overlay for frozen/operator views.
    fn finalize(mut cfg: Config, apply_environment: bool) -> Result<Config> {
        if apply_environment {
            env::apply_env(&mut cfg);
        }
        validate_team_turn_budgets(cfg.team_max_turns, cfg.team_max_sub_turns)?;
        warn_if_suspicious_model(&cfg.model);
        if let Some(root) = crate::agent::scope::current_root() {
            cfg.agent.agents_dir = Some(root);
        }
        if let Some(runtime) = crate::harness::scope::current_runtime() {
            cfg.agent.runtime = runtime;
        }
        if let Some(settings) = crate::harness::scope::current() {
            cfg.agent.codex = Some(settings);
        }
        Ok(cfg)
    }

    /// Scope-only finalizer for `load_operator` (same overrides as
    /// [`Config::load`]; the operator dir is authoritative, env is skipped).
    fn finalize_operator(cfg: Config) -> Result<Config> {
        Self::finalize(cfg, false)
    }

    /// Current effective domain value for `key` as seen from `working_dir`
    /// (project file first, else the global one). Used by the node's
    /// operator-plane bootstrap to carry the live domain view into the
    /// operator config directory exactly once.
    pub fn effective_domain_value(working_dir: &Path, key: &str) -> Option<serde_json::Value> {
        domain::read_effective_with_home(working_dir, key, None)
    }

    /// Domain file name for a domain key (`mcp_servers` -> `mcp.json`).
    pub fn domain_file_for(key: &str) -> Option<&'static str> {
        domain::domain_file_name(key)
    }

    fn load_inner(
        working_dir: &Path,
        home: Option<&Path>,
        apply_environment: bool,
    ) -> Result<Config> {
        let mut cfg = Config::default();
        // Merge ALL existing candidates, least-specific first so project files
        // override the global base (matches opencoder). This lets ~/.opencoder
        // provide the provider+key while a project opencoder.json overrides only
        // the model — `opencoder` then runs directly from any directory.
        let mut candidates = env::candidates_with_home(working_dir, home);
        candidates.reverse(); // global first, project last (wins)
        for p in candidates {
            if p.exists() {
                let raw = std::fs::read_to_string(&p)?;
                let parsed: serde_json::Value = serde_json::from_str(&raw)?;
                crate::provider::validate_protocol_patch(&parsed)?;
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
            if let Some(v) = domain::read_effective_with_home(working_dir, key, home) {
                domain::apply_domain(&mut cfg, key, &v);
            }
        }
        Self::finalize(cfg, apply_environment)
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
    /// Bare model id for `/embeddings` request bodies (provider prefix after
    /// the `/` stripped, same as `small_model_id`). Falls back to OpenAI's
    /// `text-embedding-3-small` when `embedding_model` is unset; embeddings
    /// hit [`Config::resolve_embedding_endpoint`] (primary provider unless
    /// `embedding_provider` names a dedicated one).
    pub fn embedding_model_id(&self) -> &str {
        match &self.embedding_model {
            Some(s) => s.split_once('/').map(|(_, m)| m).unwrap_or(s),
            None => "text-embedding-3-small",
        }
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
        let provider = self.provider_for(name).unwrap_or(&self.provider);
        let headers_src = match self.provider_for(name) {
            Some(p) => &p.headers,
            None => &self.provider.headers,
        };
        let headers: Vec<(String, String)> = headers_src
            .iter()
            .map(|h| (h.name.clone(), env::resolve_env(&h.value)))
            .collect();
        Ok(Endpoint {
            protocol: crate::ProviderProtocol::parse(&provider.protocol)?,
            provider: name.to_owned(),
            base_url: self.base_url_for(name),
            api_key: self.api_key_for(name)?,
            headers,
        })
    }

    /// Endpoint for `/embeddings` calls. `None` → the primary provider's
    /// [`Endpoint`](resolve_endpoint); `Some(name)` → that registered
    /// provider's base_url/api_key/headers, so a local embedding server can
    /// serve brain vectors while chat stays on the primary provider. An
    /// unregistered name is an error naming it (never a silent fallback —
    /// embeddings silently hitting the wrong server is a data-integrity bug:
    /// vectors from different models are not comparable).
    pub fn resolve_embedding_endpoint(&self) -> Result<Endpoint> {
        match &self.embedding_provider {
            Some(name) if name != self.provider_id() => {
                let p = self.provider_for(name).ok_or_else(|| {
                    CoreError::Config(format!(
                        "unknown embedding_provider `{name}`: not in the `providers` registry"
                    ))
                })?;
                let headers: Vec<(String, String)> = p
                    .headers
                    .iter()
                    .map(|h| (h.name.clone(), env::resolve_env(&h.value)))
                    .collect();
                Ok(Endpoint {
                    protocol: crate::ProviderProtocol::parse(&p.protocol)?,
                    provider: name.to_owned(),
                    base_url: p.base_url.clone(),
                    api_key: self.api_key_for(name)?,
                    headers,
                })
            }
            _ => self.resolve_endpoint(),
        }
    }

    /// Effective TUI frame rate (FPS), clamped to 1..=30. `None` -> 10.
    pub fn tui_fps(&self) -> u32 {
        self.fps.unwrap_or(10).clamp(1, 30)
    }

    /// Frame interval in milliseconds derived from [`tui_fps`](Self::tui_fps).
    pub fn tui_frame_ms(&self) -> u64 {
        1000 / self.tui_fps() as u64
    }
}

#[cfg(test)]
mod tests;
